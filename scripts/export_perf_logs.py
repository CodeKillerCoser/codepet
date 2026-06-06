#!/usr/bin/env python3

from __future__ import annotations

import argparse
import csv
import json
import math
import os
import re
import sys
from collections import defaultdict
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Iterable


LOG_LINE_RE = re.compile(
    r"^(?P<timestamp>\S+)\s+(?P<level>[A-Z]+)\s+\[(?P<target>[^\]]+)\]\s?(?P<message>.*)$"
)
BASE_LOG_COLUMNS = [
    "source_file",
    "line_number",
    "parsed",
    "timestamp",
    "level",
    "target",
    "message",
]
BASE_PERF_COLUMNS = [
    "source_file",
    "line_number",
    "timestamp",
    "name",
    "status",
    "duration_ms",
    "error",
    "raw_message",
]
SUMMARY_COLUMNS = [
    "name",
    "status",
    "count",
    "error_count",
    "avg_ms",
    "p50_ms",
    "p95_ms",
    "max_ms",
    "first_timestamp",
    "last_timestamp",
]
TIMESERIES_COLUMNS = [
    "bucket_start",
    "name",
    "status",
    "count",
    "error_count",
    "avg_ms",
    "p95_ms",
    "max_ms",
]


@dataclass(frozen=True)
class LogRecord:
    source_file: Path
    line_number: int
    parsed: bool
    timestamp: str
    level: str
    target: str
    message: str


def default_log_dir() -> Path:
    if os.name == "nt":
        root = os.environ.get("LOCALAPPDATA") or os.environ.get("APPDATA")
        if root:
            return Path(root) / "code-pet" / "logs"
    if sys.platform == "darwin":
        return Path.home() / "Library" / "Application Support" / "code-pet" / "logs"
    return Path(os.environ.get("XDG_DATA_HOME", Path.home() / ".local" / "share")) / "code-pet" / "logs"


def discover_log_files(
    explicit_logs: Iterable[Path],
    log_dir: Path | None,
    include_archives: bool,
) -> list[Path]:
    paths = [path.expanduser() for path in explicit_logs]
    if not paths:
        directory = (log_dir or default_log_dir()).expanduser()
        if include_archives:
            paths = sorted(directory.glob("code-pet*.log*"))
        else:
            paths = [directory / "code-pet.log"]

    unique_paths: list[Path] = []
    seen = set()
    for path in paths:
        resolved_key = str(path.resolve()) if path.exists() else str(path)
        if resolved_key in seen:
            continue
        seen.add(resolved_key)
        unique_paths.append(path)
    return unique_paths


def parse_log_line(line: str, source_file: Path, line_number: int) -> LogRecord:
    text = line.rstrip("\n")
    match = LOG_LINE_RE.match(text)
    if not match:
        return LogRecord(
            source_file=source_file,
            line_number=line_number,
            parsed=False,
            timestamp="",
            level="",
            target="",
            message=text,
        )

    return LogRecord(
        source_file=source_file,
        line_number=line_number,
        parsed=True,
        timestamp=match.group("timestamp"),
        level=match.group("level"),
        target=match.group("target"),
        message=match.group("message"),
    )


def read_log_file(path: Path) -> list[LogRecord]:
    records: list[LogRecord] = []
    with path.open("r", encoding="utf-8", errors="replace") as handle:
        for line_number, line in enumerate(handle, start=1):
            if not line.strip():
                continue
            records.append(parse_log_line(line, path, line_number))
    return records


def read_log_files(paths: Iterable[Path]) -> list[LogRecord]:
    records: list[LogRecord] = []
    for path in paths:
        records.extend(read_log_file(path))
    return records


def split_key_value_tokens(message: str) -> list[str]:
    tokens: list[str] = []
    current: list[str] = []
    in_string = False
    escaped = False

    for character in message:
        if escaped:
            current.append(character)
            escaped = False
            continue
        if character == "\\" and in_string:
            current.append(character)
            escaped = True
            continue
        if character == '"':
            in_string = not in_string
            current.append(character)
            continue
        if character.isspace() and not in_string:
            if current:
                tokens.append("".join(current))
                current = []
            continue
        current.append(character)

    if current:
        tokens.append("".join(current))
    return tokens


def parse_key_value_message(message: str) -> dict[str, Any]:
    fields: dict[str, Any] = {}
    for token in split_key_value_tokens(message):
        key, separator, value = token.partition("=")
        if not separator or not key:
            continue
        fields[key] = parse_value(value)
    return fields


def parse_value(value: str) -> Any:
    if value.startswith('"') and value.endswith('"'):
        try:
            return json.loads(value)
        except json.JSONDecodeError:
            return value[1:-1]
    return value


def log_event_rows(records: Iterable[LogRecord]) -> list[dict[str, Any]]:
    return [
        {
            "source_file": str(record.source_file),
            "line_number": record.line_number,
            "parsed": record.parsed,
            "timestamp": record.timestamp,
            "level": record.level,
            "target": record.target,
            "message": record.message,
        }
        for record in records
    ]


def perf_event_rows(records: Iterable[LogRecord]) -> list[dict[str, Any]]:
    rows: list[dict[str, Any]] = []
    for record in records:
        if record.target != "perf":
            continue
        fields = parse_key_value_message(record.message)
        row: dict[str, Any] = {
            "source_file": str(record.source_file),
            "line_number": record.line_number,
            "timestamp": record.timestamp,
            "name": fields.pop("name", ""),
            "status": fields.pop("status", "ok"),
            "duration_ms": to_float(fields.pop("duration_ms", "")),
            "error": fields.pop("error", ""),
            "raw_message": record.message,
        }
        for key, value in sorted(fields.items()):
            row[f"field.{key}"] = value
        rows.append(row)
    return rows


def summary_rows(rows: Iterable[dict[str, Any]]) -> list[dict[str, Any]]:
    grouped: dict[tuple[str, str], list[dict[str, Any]]] = defaultdict(list)
    for row in rows:
        grouped[(str(row.get("name", "")), str(row.get("status", "ok")))].append(row)

    summaries: list[dict[str, Any]] = []
    for (name, status), group in sorted(grouped.items()):
        durations = sorted(
            duration
            for duration in (to_float(row.get("duration_ms")) for row in group)
            if duration is not None
        )
        timestamps = sorted(str(row.get("timestamp", "")) for row in group if row.get("timestamp"))
        summaries.append(
            {
                "name": name,
                "status": status,
                "count": len(group),
                "error_count": sum(1 for row in group if row.get("status") != "ok" or row.get("error")),
                "avg_ms": rounded(sum(durations) / len(durations)) if durations else "",
                "p50_ms": rounded(percentile(durations, 0.50)) if durations else "",
                "p95_ms": rounded(percentile(durations, 0.95)) if durations else "",
                "max_ms": rounded(max(durations)) if durations else "",
                "first_timestamp": timestamps[0] if timestamps else "",
                "last_timestamp": timestamps[-1] if timestamps else "",
            }
        )
    return summaries


def timeseries_rows(rows: Iterable[dict[str, Any]], bucket_minutes: int) -> list[dict[str, Any]]:
    grouped: dict[tuple[str, str, str], list[dict[str, Any]]] = defaultdict(list)
    for row in rows:
        bucket = bucket_start(str(row.get("timestamp", "")), bucket_minutes)
        if not bucket:
            continue
        grouped[(bucket, str(row.get("name", "")), str(row.get("status", "ok")))].append(row)

    output: list[dict[str, Any]] = []
    for (bucket, name, status), group in sorted(grouped.items()):
        durations = sorted(
            duration
            for duration in (to_float(row.get("duration_ms")) for row in group)
            if duration is not None
        )
        output.append(
            {
                "bucket_start": bucket,
                "name": name,
                "status": status,
                "count": len(group),
                "error_count": sum(1 for row in group if row.get("status") != "ok" or row.get("error")),
                "avg_ms": rounded(sum(durations) / len(durations)) if durations else "",
                "p95_ms": rounded(percentile(durations, 0.95)) if durations else "",
                "max_ms": rounded(max(durations)) if durations else "",
            }
        )
    return output


def export_tables(records: list[LogRecord], out_dir: Path, bucket_minutes: int = 1) -> dict[str, int]:
    out_dir.mkdir(parents=True, exist_ok=True)
    logs = log_event_rows(records)
    perf = perf_event_rows(records)
    summaries = summary_rows(perf)
    series = timeseries_rows(perf, bucket_minutes)

    write_csv(out_dir / "log_events.csv", logs, BASE_LOG_COLUMNS)
    write_csv(out_dir / "perf_events.csv", perf, BASE_PERF_COLUMNS)
    write_csv(out_dir / "perf_summary.csv", summaries, SUMMARY_COLUMNS)
    write_csv(out_dir / "perf_timeseries.csv", series, TIMESERIES_COLUMNS)

    return {
        "log_events.csv": len(logs),
        "perf_events.csv": len(perf),
        "perf_summary.csv": len(summaries),
        "perf_timeseries.csv": len(series),
    }


def write_csv(path: Path, rows: list[dict[str, Any]], preferred_columns: list[str]) -> None:
    extra_columns = sorted({key for row in rows for key in row if key not in preferred_columns})
    fieldnames = preferred_columns + extra_columns
    with path.open("w", encoding="utf-8", newline="") as handle:
        writer = csv.DictWriter(handle, fieldnames=fieldnames)
        writer.writeheader()
        for row in rows:
            writer.writerow({key: csv_value(row.get(key, "")) for key in fieldnames})


def csv_value(value: Any) -> Any:
    if value is None:
        return ""
    if isinstance(value, (dict, list)):
        return json.dumps(value, ensure_ascii=False, separators=(",", ":"))
    return value


def to_float(value: Any) -> float | None:
    if value is None or value == "":
        return None
    try:
        return float(value)
    except (TypeError, ValueError):
        return None


def percentile(values: list[float], quantile: float) -> float:
    if not values:
        raise ValueError("percentile requires at least one value")
    index = max(0, min(len(values) - 1, math.ceil(len(values) * quantile) - 1))
    return values[index]


def rounded(value: float) -> float:
    return round(value, 3)


def parse_timestamp(value: str) -> datetime | None:
    if not value:
        return None
    try:
        return datetime.fromisoformat(value.replace("Z", "+00:00"))
    except ValueError:
        return None


def bucket_start(timestamp: str, bucket_minutes: int) -> str:
    parsed = parse_timestamp(timestamp)
    if not parsed:
        return ""
    bucket_seconds = max(1, bucket_minutes) * 60
    epoch_seconds = int(parsed.timestamp())
    bucket_epoch = epoch_seconds - (epoch_seconds % bucket_seconds)
    tzinfo = parsed.tzinfo or timezone.utc
    return datetime.fromtimestamp(bucket_epoch, tzinfo).isoformat(timespec="minutes")


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        description="Clean Code Pet log files and export CSV tables for performance analysis."
    )
    parser.add_argument(
        "--log",
        action="append",
        default=[],
        type=Path,
        help="Log file to parse. Can be passed multiple times.",
    )
    parser.add_argument(
        "--log-dir",
        type=Path,
        help="Directory containing code-pet.log. Defaults to the platform app data log directory.",
    )
    parser.add_argument(
        "--include-archives",
        action="store_true",
        help="Include rotated code-pet log archives from --log-dir.",
    )
    parser.add_argument(
        "--out",
        type=Path,
        default=Path("perf-log-tables"),
        help="Output directory for CSV tables.",
    )
    parser.add_argument(
        "--bucket-minutes",
        type=int,
        default=1,
        help="Aggregation window for perf_timeseries.csv.",
    )
    return parser


def main(argv: list[str] | None = None) -> int:
    args = build_parser().parse_args(argv)
    paths = discover_log_files(args.log, args.log_dir, args.include_archives)
    missing = [path for path in paths if not path.exists()]
    existing = [path for path in paths if path.exists()]
    for path in missing:
        print(f"warning: log file not found: {path}", file=sys.stderr)
    if not existing:
        print("no log files found", file=sys.stderr)
        return 2

    records = read_log_files(existing)
    counts = export_tables(records, args.out, args.bucket_minutes)
    for filename, row_count in counts.items():
        print(f"wrote {args.out / filename} rows={row_count}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
