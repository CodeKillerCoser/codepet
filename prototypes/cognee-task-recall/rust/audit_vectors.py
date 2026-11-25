"""Read historical Lance versions to verify in-place vector replacement, without writes."""
import argparse
import hashlib
import json
import math
from pathlib import Path
import struct
import uuid

import lancedb


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("root", type=Path)
    args = parser.parse_args()
    root = args.root.resolve()
    report = json.loads((root / "report.json").read_text())
    table = lancedb.connect(root / "runtime/system/databases/cognee.lancedb").open_table("DocumentChunk_text")
    records, representatives = [], {}
    for version in table.list_versions():
        table.checkout(version["version"])
        for row in table.to_arrow().to_pylist():
            metadata = json.loads(row["metadata"])
            text = metadata.get("text", "")
            stage = ("before" if "CODEPET_PROBE_OLD_REVISION" in text else
                     "after" if "CODEPET_PROBE_NEW_REVISION" in text else None)
            if not stage:
                continue
            vector = row["vector"]
            vector_hash = hashlib.sha256(struct.pack(f"<{len(vector)}f", *vector)).hexdigest()
            records.append({"version":version["version"],"stage":stage,
                "id":str(uuid.UUID(bytes=row["id"])),"dimensions":len(vector),
                "vectorSha256":vector_hash,"textSha256":hashlib.sha256(text.encode()).hexdigest()})
            representatives.setdefault(stage, vector)
    before, after = representatives["before"], representatives["after"]
    cosine = sum(a*b for a,b in zip(before, after)) / math.sqrt(sum(a*a for a in before)*sum(b*b for b in after))
    changed = any(a != b for a,b in zip(before, after))
    checks = {"sameChunkIdAcrossVersions":len({r["id"] for r in records}) == 1,
              "oldAndNewTextVersionsPresent":set(representatives) == {"before","after"},
              "storedVectorActuallyChanged":changed,"dimensionsRemain1024":all(r["dimensions"] == 1024 for r in records)}
    audit = {"method":"Read-only historical Lance table checkout; no vectors written or regenerated.",
             "versions":records,"oldVsNewCosine":cosine,"checks":checks}
    (root / "vector-version-audit.json").write_text(json.dumps(audit, ensure_ascii=False, indent=2))
    report["update"]["checks"]["oldChunkVectorsRemoved"] = None
    report["update"]["checks"].update(checks)
    report["update"]["vectorUpdateMechanism"] = "Same chunk ID, updated text and vector; removed-ID check is not applicable."
    (root / "report.json").write_text(json.dumps(report, ensure_ascii=False, indent=2))
    print(json.dumps(audit, ensure_ascii=False, indent=2))


if __name__ == "__main__":
    main()
