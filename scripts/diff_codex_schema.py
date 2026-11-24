#!/usr/bin/env python3
"""Development-only structural diff of two official generate-json-schema exports.

python3 scripts/diff_codex_schema.py OLD_EXPORT NEW_EXPORT > report.json
Exports include local definitions, so a referenced DTO change is reported alongside
its request/response. No production dependency or automatic compatibility verdict.
"""
import argparse
import hashlib
import json
from pathlib import Path

ANNOTATIONS = {"description", "title", "$id", "$schema", "examples"}


def contract(value):
    if isinstance(value, dict):
        return {key: contract(item) for key, item in value.items() if key not in ANNOTATIONS}
    if isinstance(value, list):
        return [contract(item) for item in value]
    return value


def changes(before, after, path=""):
    if isinstance(before, dict) and isinstance(after, dict):
        result = []
        for key in sorted(before.keys() | after.keys()):
            pointer = path + "/" + key.replace("~", "~0").replace("/", "~1")
            if key not in before:
                result.append({"path": pointer, "change": "added", "after": after[key]})
            elif key not in after:
                result.append({"path": pointer, "change": "removed", "before": before[key]})
            else:
                result.extend(changes(before[key], after[key], pointer))
        return result
    if before == after:
        return []
    return [{"path": path, "change": "changed", "before": before, "after": after}]


def export(directory):
    files = {str(path.relative_to(directory)): json.loads(path.read_text())
             for path in sorted(directory.rglob("*.json"))}
    if not files:
        raise ValueError(f"no schema JSON found in {directory}")
    digest = hashlib.sha256(json.dumps(files, sort_keys=True, separators=(",", ":")).encode()).hexdigest()
    return files, digest


def compare(old, new):
    before, old_hash = export(old)
    after, new_hash = export(new)
    result = {}
    for name in sorted(before.keys() | after.keys()):
        diff = changes(contract(before.get(name)), contract(after.get(name)))
        if diff:
            result[name] = diff
    return {"baselineSha256": old_hash, "currentSha256": new_hash, "changes": result}


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("baseline", type=Path)
    parser.add_argument("current", type=Path)
    args = parser.parse_args()
    print(json.dumps(compare(args.baseline, args.current), ensure_ascii=False, indent=2))
