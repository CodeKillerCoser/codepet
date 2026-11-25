"""Measure release artifacts; build caches and external model weights are excluded."""
import argparse
import gzip
import hashlib
import json
from pathlib import Path
import subprocess
import tarfile
import time


def measure(path):
    data = path.read_bytes()
    deps = subprocess.check_output(["otool", "-L", str(path)], text=True)
    libraries = [line.strip().split(" (", 1)[0] for line in deps.splitlines()[1:]]
    external = [p for p in libraries if not p.startswith(("/usr/lib/", "/System/Library/"))]
    return {"file": path.name, "bytes":len(data), "gzipBytes":len(gzip.compress(data, mtime=0)),
            "sha256":hashlib.sha256(data).hexdigest(), "dynamicLibraries":libraries,
            "nonSystemDynamicLibraries":external}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("target", type=Path)
    parser.add_argument("source", type=Path)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    binary = args.target / "release/cognee-cli"
    driver = args.target / "release/codepet-probe"
    cli = measure(binary)
    samples = []
    for _ in range(3):
        start = time.perf_counter()
        subprocess.run([str(binary), "--version"], check=True, capture_output=True)
        samples.append(time.perf_counter() - start)
    cli["versionCommandSeconds"] = samples
    tree = subprocess.check_output(["cargo", "+stable", "tree", "-p", "cognee-cli", "--no-default-features",
                                   "--features", "ladybug,lancedb,sqlite,tiktoken,visualization", "--prefix", "none"],
                                   cwd=args.source, text=True)
    args.output.mkdir(parents=True, exist_ok=True)
    (args.output / "cargo-tree.txt").write_text(tree)
    archive = args.output / "cognee-rs-external-models-macos-arm64.tar.gz"
    with tarfile.open(archive, "w:gz") as package:
        package.add(binary, arcname="cognee-rs/cognee-cli")
        for license in ("LICENSE-MIT", "LICENSE-APACHE"):
            package.add(args.source / license, arcname="cognee-rs/" + license)
    result = {"platform":"macOS arm64", "commit":"c3165875027bfff353f407d031b60fecad0c8b96",
              "features":["ladybug","lancedb","sqlite","tiktoken","visualization"],
              "defaultFeatures":False, "releaseDebug":0, "strip":"symbols",
              "onnxRuntimeInDependencyTree":any(line.startswith(("ort v", "ort-sys v")) for line in tree.splitlines()),
              "modelWeightsIncluded":False, "modelProvider":"external HTTP API",
              "cli":cli, "experimentDriver":measure(driver),
              "cliArchive":{"file":archive.name,"bytes":archive.stat().st_size,
                            "contents":["official CLI", "MIT and Apache licenses"]},
              "excluded":["Cargo build cache", "runtime databases", "Python test harness", "MLX app and installed models"],
              "limitations":["gzip is a compressed executable, not a signed installer.",
                             "--version startup is not pipeline initialization or recall latency.",
                             "Rust direct linking into CodePet may have a different incremental size."]}
    (args.output / "package-measurement.json").write_text(json.dumps(result, ensure_ascii=False, indent=2))
    print(json.dumps(result, ensure_ascii=False, indent=2))


if __name__ == "__main__":
    main()
