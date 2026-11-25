"""Durable desired versions, retry queue and Chinese-capable SQLite FTS5."""
from __future__ import annotations

import hashlib
import json
from pathlib import Path
import re
import sqlite3
import threading
import time
import uuid
from .history import HistoryMixin


def tokens(text):
    result = []
    for term in re.findall(r"[a-z0-9_]+|[\u3400-\u9fff]+", text.casefold()):
        if re.fullmatch(r"[\u3400-\u9fff]+", term) and len(term) > 1:
            result.extend(term[i:i + 2] for i in range(len(term) - 1))
        else:
            result.append(term)
    return result


def validate_document(doc):
    for key in ("runId", "taskId"):
        uuid.UUID(doc[key])
    if not isinstance(doc.get("text"), str) or len(doc["text"].encode()) > 512 * 1024:
        raise ValueError("Invalid document text")
    expected = hashlib.sha256(doc["text"].encode()).hexdigest()
    if doc.get("contentHash") != expected:
        raise ValueError("Document hash mismatch")
    return doc


class Catalog(HistoryMixin):
    def __init__(self, root: Path, models=None, usage_tracking=True):
        root.mkdir(parents=True, exist_ok=True)
        self.lock = threading.RLock()
        self.db = sqlite3.connect(root / "catalog.sqlite", check_same_thread=False)
        self.db.row_factory = sqlite3.Row
        self.db.executescript("""
            PRAGMA journal_mode=WAL;
            PRAGMA busy_timeout=5000;
            CREATE TABLE IF NOT EXISTS runs(
                run_id TEXT PRIMARY KEY, document TEXT NOT NULL, desired_hash TEXT NOT NULL,
                indexed_hash TEXT, data_id TEXT, eligible INTEGER NOT NULL, deleted INTEGER NOT NULL DEFAULT 0,
                state TEXT NOT NULL DEFAULT 'pending', attempts INTEGER NOT NULL DEFAULT 0,
                retry_at REAL NOT NULL DEFAULT 0, error TEXT);
            CREATE VIRTUAL TABLE IF NOT EXISTS run_fts USING fts5(run_id UNINDEXED, terms);
        """)
        self.init_history(models, usage_tracking)

    def sync(self, documents):
        if len(documents) > 10000:
            raise ValueError("Too many Run documents")
        docs = [validate_document(dict(doc)) for doc in documents]
        ids = [d["runId"] for d in docs]
        if len(set(ids)) != len(ids):
            raise ValueError("Duplicate Run IDs")
        with self.lock, self.db:
            existing = {r["run_id"]:dict(r) for r in self.db.execute("SELECT * FROM runs")}
            for doc in docs:
                old = existing.get(doc["runId"])
                changed = not old or old["desired_hash"] != doc["contentHash"] or old["deleted"]
                eligible = bool(doc.get("eligible"))
                state = "pending" if changed else old["state"]
                # Progress drafts must not serve stale semantic results from an older finished version.
                if not eligible:
                    state = "draft"
                elif old and old["state"] == "draft":
                    state = "pending"
                job = old["job_id"] if old else None
                if changed or (old and old["state"] == "draft" and eligible):
                    self.retire_queued(job)
                    job = self.enqueue(doc) if eligible else None
                elif not eligible:
                    self.retire_queued(job, "执行尚未完成")
                    job = None
                self.db.execute("""INSERT INTO runs(run_id,document,desired_hash,eligible,state)
                    VALUES(?,?,?,?,?) ON CONFLICT(run_id) DO UPDATE SET document=excluded.document,
                    desired_hash=excluded.desired_hash, eligible=excluded.eligible, deleted=0,
                    state=excluded.state, retry_at=CASE WHEN desired_hash!=excluded.desired_hash THEN 0 ELSE retry_at END,
                    rebuild=CASE WHEN desired_hash!=excluded.desired_hash THEN 0 ELSE rebuild END,
                    attempts=CASE WHEN desired_hash!=excluded.desired_hash THEN 0 ELSE attempts END""",
                    (doc["runId"], json.dumps(doc, ensure_ascii=False), doc["contentHash"], eligible, state))
                self.db.execute("UPDATE runs SET job_id=? WHERE run_id=?", (job, doc["runId"]))
                if changed:
                    self.db.execute("DELETE FROM run_fts WHERE run_id=?", (doc["runId"],))
                    self.db.execute("INSERT INTO run_fts VALUES(?,?)", (doc["runId"], " ".join(tokens(doc["text"]))))
            for missing in {key for key in set(existing) - set(ids) if not existing[key]["deleted"]}:
                self.retire_queued(existing[missing]["job_id"], "原始执行已删除")
                self.db.execute("UPDATE runs SET deleted=1,state='pending',retry_at=0,job_id=NULL WHERE run_id=?", (missing,))
                self.db.execute("DELETE FROM run_fts WHERE run_id=?", (missing,))
        return self.status()

    def status(self):
        with self.lock:
            counts = dict(self.db.execute("SELECT state,count(*) FROM runs WHERE deleted=0 GROUP BY state"))
        return {"counts":counts, "pending":sum(counts.get(k, 0) for k in ("pending", "processing", "failed")),
                "ready":counts.get("ready", 0)}

    def claim(self):
        with self.lock, self.db:
            row = self.db.execute("""SELECT * FROM runs WHERE (eligible=1 OR deleted=1)
                AND state IN ('pending','failed') AND retry_at<=? ORDER BY attempts,run_id LIMIT 1""", (time.time(),)).fetchone()
            if row:
                row = dict(row)
                row["job_id"] = self.start_attempt(row)
                self.db.execute("UPDATE runs SET state='processing',job_id=? WHERE run_id=?", (row["job_id"], row["run_id"]))
                return row

    def finish(self, row, data_id=None, error=None, result=None):
        with self.lock, self.db:
            current = self.db.execute("SELECT * FROM runs WHERE run_id=?", (row["run_id"],)).fetchone()
            if not current:
                self.end_attempt(row, "superseded", result)
                return
            if (current["desired_hash"] != row["desired_hash"] or current["deleted"] != row["deleted"]
                    or current["eligible"] != row["eligible"]):
                state = current["state"] if current["job_id"] != row.get("job_id") else ("pending" if current["eligible"] or current["deleted"] else "draft")
                self.db.execute("UPDATE runs SET data_id=COALESCE(?,data_id),state=? WHERE run_id=?", (data_id,state,row["run_id"]))
                self.end_attempt(row, "superseded", result)
            elif error:
                attempts = current["attempts"] + 1
                self.db.execute("UPDATE runs SET state='failed',error=?,attempts=?,retry_at=? WHERE run_id=?",
                                (error,attempts,time.time() + min(300, 2 ** min(attempts, 8)),row["run_id"]))
                self.end_attempt(row, "failed", error=error)
                if not row["deleted"]:
                    job = self.enqueue(json.loads(row["document"]))
                    self.db.execute("UPDATE runs SET job_id=? WHERE run_id=?", (job, row["run_id"]))
            elif row["deleted"]:
                self.db.execute("DELETE FROM runs WHERE run_id=?", (row["run_id"],))
            else:
                self.db.execute("UPDATE runs SET state='ready',data_id=?,indexed_hash=?,error=NULL,attempts=0,rebuild=0 WHERE run_id=?",
                                (data_id,row["desired_hash"],row["run_id"]))
                self.end_attempt(row, "completed", result)

    def ready(self):
        with self.lock:
            return {r["run_id"]:json.loads(r["document"]) for r in self.db.execute(
                "SELECT * FROM runs WHERE deleted=0 AND eligible=1 AND state='ready' AND indexed_hash=desired_hash")}

    def lexical(self, query):
        terms = list(dict.fromkeys(tokens(query)))[:40]
        if not terms:
            return []
        expression = " OR ".join('"' + term.replace('"', '""') + '"' for term in terms)
        with self.lock:
            rows = self.db.execute("""SELECT runs.document,bm25(run_fts) AS score FROM run_fts
                JOIN runs ON runs.run_id=run_fts.run_id WHERE run_fts MATCH ? AND runs.deleted=0
                ORDER BY score, runs.run_id LIMIT 40""", (expression,)).fetchall()
        return [{"runId":json.loads(row["document"])["runId"], "score":-row["score"],
                 "contentHash":json.loads(row["document"])["contentHash"]} for row in rows]
