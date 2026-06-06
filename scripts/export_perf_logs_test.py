#!/usr/bin/env python3

from __future__ import annotations

import csv
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "scripts"))

import export_perf_logs


class PerfLogExportTest(unittest.TestCase):
    def test_parses_structured_log_line(self):
        record = export_perf_logs.parse_log_line(
            "2026-06-06T12:00:00.000+08:00 INFO  [app] logging initialized path=C:/tmp/code-pet.log",
            Path("code-pet.log"),
            1,
        )

        self.assertTrue(record.parsed)
        self.assertEqual(record.timestamp, "2026-06-06T12:00:00.000+08:00")
        self.assertEqual(record.level, "INFO")
        self.assertEqual(record.target, "app")
        self.assertEqual(record.message, "logging initialized path=C:/tmp/code-pet.log")

    def test_parses_perf_key_value_message_with_quoted_values(self):
        fields = export_perf_logs.parse_key_value_message(
            'name=frontend.main.refresh status=error duration_ms=42 error="IPC unavailable" window="main window"'
        )

        self.assertEqual(fields["name"], "frontend.main.refresh")
        self.assertEqual(fields["status"], "error")
        self.assertEqual(fields["duration_ms"], "42")
        self.assertEqual(fields["error"], "IPC unavailable")
        self.assertEqual(fields["window"], "main window")

    def test_exports_perf_event_summary_and_timeseries_tables(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp = Path(temp_dir)
            log_path = temp / "code-pet.log"
            out_dir = temp / "tables"
            log_path.write_text(
                "\n".join(
                    [
                        "2026-06-06T12:00:00.000+08:00 INFO  [app] logging initialized",
                        "2026-06-06T12:00:01.000+08:00 INFO  [perf] name=frontend.pet.sync_recent_events status=ok duration_ms=120 events=4",
                        "2026-06-06T12:00:20.000+08:00 INFO  [perf] name=frontend.pet.sync_recent_events status=ok duration_ms=80 events=0",
                        '2026-06-06T12:01:02.000+08:00 INFO  [perf] name=frontend.main.refresh status=error duration_ms=10 error="IPC unavailable"',
                    ]
                )
                + "\n",
                encoding="utf-8",
            )

            records = export_perf_logs.read_log_files([log_path])
            counts = export_perf_logs.export_tables(records, out_dir, bucket_minutes=1)

            self.assertEqual(counts["log_events.csv"], 4)
            self.assertEqual(counts["perf_events.csv"], 3)
            perf_rows = read_csv(out_dir / "perf_events.csv")
            self.assertEqual(perf_rows[0]["field.events"], "4")
            summary_rows = read_csv(out_dir / "perf_summary.csv")
            sync_summary = next(row for row in summary_rows if row["name"] == "frontend.pet.sync_recent_events")
            self.assertEqual(sync_summary["count"], "2")
            self.assertEqual(sync_summary["avg_ms"], "100.0")
            self.assertEqual(sync_summary["p95_ms"], "120.0")
            refresh_summary = next(row for row in summary_rows if row["name"] == "frontend.main.refresh")
            self.assertEqual(refresh_summary["error_count"], "1")
            timeseries_rows = read_csv(out_dir / "perf_timeseries.csv")
            self.assertEqual(len(timeseries_rows), 2)
            self.assertEqual(timeseries_rows[0]["bucket_start"], "2026-06-06T12:00+08:00")

    def test_discovers_current_log_only_by_default(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            log_dir = Path(temp_dir)
            current = log_dir / "code-pet.log"
            archived = log_dir / "code-pet.2026-06-05.log"
            current.write_text("", encoding="utf-8")
            archived.write_text("", encoding="utf-8")

            self.assertEqual(
                export_perf_logs.discover_log_files([], log_dir, include_archives=False),
                [current],
            )
            self.assertEqual(
                set(export_perf_logs.discover_log_files([], log_dir, include_archives=True)),
                {current, archived},
            )


def read_csv(path: Path) -> list[dict[str, str]]:
    with path.open("r", encoding="utf-8", newline="") as handle:
        return list(csv.DictReader(handle))


if __name__ == "__main__":
    unittest.main()
