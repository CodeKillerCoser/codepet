"""Durable processing attempts. Queue actions compare both attempt and document version."""
import json
from pathlib import Path
import sqlite3
import time
import uuid

TERMINAL = {"completed", "failed", "cancelled", "superseded", "interrupted"}


class QueueConflict(Exception):
    pass


class HistoryMixin:
    def init_history(self, models, usage_tracking):
        self.models = models or {}
        self.usage_tracking = usage_tracking
        columns = {row[1] for row in self.db.execute("PRAGMA table_info(runs)")}
        if "job_id" not in columns:
            self.db.execute("ALTER TABLE runs ADD COLUMN job_id TEXT")
        if "rebuild" not in columns:
            self.db.execute("ALTER TABLE runs ADD COLUMN rebuild INTEGER NOT NULL DEFAULT 0")
        self.db.executescript("""
            CREATE TABLE IF NOT EXISTS manual_requests(
                id TEXT PRIMARY KEY, task_id TEXT NOT NULL, receipt TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS processing_jobs(
                id TEXT PRIMARY KEY, run_id TEXT NOT NULL, content_hash TEXT NOT NULL,
                document TEXT NOT NULL, models TEXT NOT NULL, state TEXT NOT NULL,
                queued_at REAL, started_at REAL, finished_at REAL, phase TEXT NOT NULL DEFAULT '',
                result TEXT, error TEXT, usage_tracking INTEGER NOT NULL DEFAULT 1);
            CREATE INDEX IF NOT EXISTS processing_jobs_run ON processing_jobs(run_id);
        """)
        now = time.time()
        with self.lock, self.db:
            # Preserve the interrupted attempt, then enqueue recovery as a NEW attempt.
            self.db.execute("UPDATE processing_jobs SET state='interrupted',finished_at=?,phase='进程已退出' WHERE state='processing'", (now,))
            self.db.execute("UPDATE runs SET state='pending',job_id=NULL WHERE state='processing'")
            for row in self.db.execute("SELECT * FROM runs WHERE deleted=0 AND eligible=1").fetchall():
                if row["job_id"]:
                    continue
                doc = json.loads(row["document"])
                if row["state"] == "ready":
                    # Older catalogs never recorded time, tokens or a result snapshot.
                    job = self.enqueue(doc, state="completed", legacy=True)
                elif row["state"] in ("pending", "failed"):
                    job = self.enqueue(doc)
                else:
                    continue
                self.db.execute("UPDATE runs SET job_id=? WHERE run_id=?", (job, row["run_id"]))

    def enqueue(self, doc, state="queued", legacy=False):
        job = str(uuid.uuid4())
        self.db.execute("INSERT INTO processing_jobs(id,run_id,content_hash,document,models,state,queued_at,usage_tracking) VALUES(?,?,?,?,?,?,?,?)",
            (job, doc["runId"], doc["contentHash"], json.dumps(doc, ensure_ascii=False),
             json.dumps(self.models, ensure_ascii=False), state, None if legacy else time.time(), self.usage_tracking and not legacy))
        return job

    def retire_queued(self, job, reason="文档已更新"):
        if job:
            self.db.execute("UPDATE processing_jobs SET state='superseded',finished_at=?,phase=? WHERE id=? AND state='queued'", (time.time(), reason, job))

    def enqueue_task(self, task_id, operation_id):
        uuid.UUID(task_id)
        uuid.UUID(operation_id)
        with self.lock, self.db:
            previous = self.db.execute("SELECT * FROM manual_requests WHERE id=?", (operation_id,)).fetchone()
            if previous:
                if previous["task_id"] != task_id:
                    raise QueueConflict("操作 ID 已用于另一个任务")
                return json.loads(previous["receipt"])
            rows = self.db.execute("SELECT * FROM runs WHERE deleted=0 AND eligible=1 AND json_extract(document,'$.taskId')=?", (task_id,)).fetchall()
            if not rows:
                raise QueueConflict("任务没有可加工的已结束执行")
            queued, existing, jobs = 0, 0, []
            for row in rows:
                if row["state"] in ("pending", "processing", "failed") and row["job_id"]:
                    if row["state"] == "failed":
                        self.db.execute("UPDATE runs SET retry_at=0 WHERE run_id=?", (row["run_id"],))
                    existing += 1
                    jobs.append(row["job_id"])
                    continue
                job = self.enqueue(json.loads(row["document"]))
                self.db.execute("UPDATE processing_jobs SET phase='等待加工（手动触发）' WHERE id=?", (job,))
                self.db.execute("UPDATE runs SET state='pending',job_id=?,retry_at=0,attempts=0,error=NULL,rebuild=? WHERE run_id=?",
                    (job, bool(row["data_id"]) or row["rebuild"], row["run_id"]))
                queued += 1
                jobs.append(job)
            receipt = {"taskId":task_id, "queued":queued, "existing":existing, "jobIds":jobs}
            self.db.execute("INSERT INTO manual_requests VALUES(?,?,?)", (operation_id, task_id, json.dumps(receipt)))
            return receipt

    def start_attempt(self, row):
        job = row["job_id"]
        if row["deleted"]:
            return None
        if not job:
            job = self.enqueue(json.loads(row["document"]))
        updated = self.db.execute("UPDATE processing_jobs SET state='processing',started_at=?,phase='加工文档与实体' WHERE id=? AND state='queued'", (time.time(), job))
        if updated.rowcount != 1:
            raise QueueConflict("加工队列状态已变化")
        return job

    def phase(self, job, phase):
        if job:
            with self.lock, self.db:
                self.db.execute("UPDATE processing_jobs SET phase=? WHERE id=? AND state='processing'", (phase, job))

    def end_attempt(self, row, state, result=None, error=None):
        if row.get("job_id"):
            self.db.execute("UPDATE processing_jobs SET state=?,finished_at=?,phase=?,result=?,error=? WHERE id=? AND state='processing'",
                (state, time.time(), {"completed":"加工完成", "failed":"加工失败", "superseded":"文档已更新"}.get(state, state),
                 json.dumps(result, ensure_ascii=False) if result is not None else None, error, row["job_id"]))

    def cancel(self, job):
        uuid.UUID(job)
        with self.lock, self.db:
            row = self.db.execute("SELECT * FROM processing_jobs WHERE id=?", (job,)).fetchone()
            current = self.db.execute("SELECT * FROM runs WHERE job_id=?", (job,)).fetchone()
            if (not row or row["state"] != "queued" or not current or current["deleted"]
                    or current["state"] not in ("pending", "failed") or current["desired_hash"] != row["content_hash"]):
                raise QueueConflict("任务已开始加工或队列已变化，请刷新后重试")
            self.db.execute("UPDATE processing_jobs SET state='cancelled',finished_at=?,phase='用户取消排队' WHERE id=?", (time.time(), job))
            self.db.execute("UPDATE runs SET state='cancelled' WHERE job_id=?", (job,))
        return {"id":job, "state":"cancelled"}

    def retry(self, job):
        uuid.UUID(job)
        with self.lock, self.db:
            old = self.db.execute("SELECT * FROM processing_jobs WHERE id=?", (job,)).fetchone()
            if not old or old["state"] not in ("failed", "cancelled", "interrupted"):
                raise QueueConflict("此记录不可重试")
            row = self.db.execute("SELECT * FROM runs WHERE run_id=?", (old["run_id"],)).fetchone()
            if not row or row["deleted"] or not row["eligible"] or row["desired_hash"] != old["content_hash"]:
                raise QueueConflict("原始执行已删除或更新，请刷新")
            if row["state"] in ("pending", "processing", "ready"):
                raise QueueConflict("此版本已有排队、加工或完成的记录")
            self.retire_queued(row["job_id"], "用户手动重试")
            new = self.enqueue(json.loads(row["document"]))
            self.db.execute("UPDATE runs SET state='pending',retry_at=0,attempts=0,job_id=? WHERE run_id=?", (new, row["run_id"]))
        return {"id":new, "state":"queued"}


def present(row, fingerprint, active, detail=False):
    doc = json.loads(row["document"])
    summary = doc.get("summary", {})
    state = row["state"]
    if not active and state in ("queued", "processing"):
        state = "paused"
    started, finished = row["started_at"], row["finished_at"]
    elapsed = None
    if started is not None and (finished is not None or state == "processing"):
        elapsed = round(max(0, (finished or time.time()) - started) * 1000)
    result = json.loads(row["result"]) if row["result"] else None
    value = {"id":row["id"], "runId":row["run_id"], "taskId":doc["taskId"],
        "title":summary.get("title") or doc.get("taskTitle") or row["run_id"],
        "taskTitle":doc.get("taskTitle", ""), "workspace":doc.get("workspace", ""),
        "contentHash":row["content_hash"], "fingerprint":fingerprint, "state":state,
        "queuedAt":round(row["queued_at"] * 1000) if row["queued_at"] is not None else None,
        "startedAt":round(started * 1000) if started is not None else None,
        "finishedAt":round(finished * 1000) if finished is not None else None,
        "elapsedMs":elapsed, "phase":"模型配置已切换" if state == "paused" else row["phase"],
        "error":row["error"], "models":json.loads(row["models"]), "usageTracked":bool(row["usage_tracking"]),
        "resultSummary": result.get("description", "") if result else "",
        "canCancel":active and state == "queued",
        "canRetry":active and state in ("failed", "cancelled", "interrupted")}
    if detail:
        value["result"] = result
        value["legacy"] = not row["usage_tracking"]
    return value


def read_history(root, current_fingerprint, *, job=None, offset=0, limit=40, state=None):
    if job:
        uuid.UUID(job)
    if (not isinstance(offset, int) or not isinstance(limit, int) or not 0 <= offset <= 10000
            or not 1 <= limit <= 100 or state not in (None, "queued", "processing", "completed", "failed", "cancelled", "paused", "interrupted", "superseded")):
        raise ValueError("Invalid history pagination")
    items, counts = [], {}
    for path in sorted(Path(root).glob("*/catalog.sqlite")):
        # Read only known catalog locations. No user-controlled database paths.
        db = sqlite3.connect(path.resolve().as_uri() + "?mode=ro", uri=True, timeout=5)
        db.row_factory = sqlite3.Row
        try:
            if not db.execute("SELECT 1 FROM sqlite_master WHERE name='processing_jobs'").fetchone():
                continue
            fingerprint, active = path.parent.name, path.parent.name == current_fingerprint
            for status, count in db.execute("SELECT state,count(*) FROM processing_jobs GROUP BY state"):
                status = "paused" if not active and status in ("queued", "processing") else status
                counts[status] = counts.get(status, 0) + count
            if job:
                rows = db.execute("SELECT * FROM processing_jobs WHERE id=?", (job,)).fetchall()
            else:
                statuses = [state] if state else None
                if state == "paused":
                    statuses = ["queued", "processing"] if not active else []
                elif not active and state in ("queued", "processing"):
                    statuses = []
                if statuses == []:
                    continue
                where = "WHERE state IN (" + ",".join("?" for _ in statuses) + ")" if statuses else ""
                rows = db.execute(f"SELECT * FROM processing_jobs {where} ORDER BY coalesce(queued_at,0) DESC,id DESC LIMIT ?", [*(statuses or []), offset + limit]).fetchall()
            items.extend(present(row, fingerprint, active, bool(job)) for row in rows)
        finally:
            db.close()
    if job:
        if not items:
            raise QueueConflict("加工记录不存在")
        return items[0]
    items.sort(key=lambda row:(row["queuedAt"] or 0, row["id"]), reverse=True)
    total = counts.get(state, 0) if state else sum(counts.values())
    return {"items":items[offset:offset + limit], "total":total, "counts":counts,
            "offset":offset, "hasMore":offset + limit < total}
