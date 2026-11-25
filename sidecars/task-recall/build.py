"""Native directory bundle: uv standalone CPython + hash-locked Cognee, no model weights."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import plistlib
import shutil
import subprocess
import tempfile
import tarfile

ROOT = Path(__file__).resolve().parent


def run(*args, **kwargs):
    return subprocess.run(args, check=True, **kwargs)


def patch_cognee(packages):
    # Cognee 1.6.10's lease proxy hides include_vector from signature-based
    # capability detection, causing the native Semantic view to re-embed.
    relative = "cognee/infrastructure/databases/utils/closing_lru_cache.py"
    path = packages / relative
    source = path.read_text(encoding="utf-8")
    needle = "        def call_with_lease(*args, **kwargs):"
    if source.count(needle) != 1:
        raise RuntimeError("Cognee lease-proxy patch no longer matches; review the pinned framework")
    patched = source.replace(needle, "        # CodePet compatibility fix: preserve native capability signatures.\n        @wraps(attr)\n" + needle)
    path.write_text(patched, encoding="utf-8")
    return {"file":relative,"reason":"Preserve leased method signatures for native stored-vector visualization",
            "upstreamSha256":hashlib.sha256(source.encode()).hexdigest(),"patchedSha256":hashlib.sha256(patched.encode()).hexdigest()}


def stage(bundle, destination):
    if destination is None:
        return
    destination = destination.resolve()
    destination.mkdir(parents=True, exist_ok=True)
    info = json.loads((bundle / "manifest.json").read_text())
    descriptor = destination / "bundle.json"
    archive = destination / "bundle.tar.gz"
    if descriptor.exists() and archive.exists() and json.loads(descriptor.read_text()).get("sourceHash") == info["sourceHash"]:
        return
    temporary = destination / "bundle.tar.gz.tmp"
    # Opaque archive prevents the App bundler from re-signing Python libraries
    # after their checksums have been recorded. Rust expands it on installation.
    with tarfile.open(temporary, "w:gz", compresslevel=6) as tar:
        for file in sorted(bundle.rglob("*")):
            if file.is_file():
                tar.add(file, arcname=file.relative_to(bundle).as_posix(), recursive=False)
    digest = hashlib.file_digest(open(temporary, "rb"), "sha256").hexdigest()
    os.replace(temporary, archive)
    # Remove a preceding directory-style stage only after the replacement archive exists.
    if destination != bundle.resolve():
        for name in ("runtime","packages","codepet_recall","tokenizer-cache","main.py","uv.lock","manifest.json"):
            old = destination / name
            if old.is_dir():
                shutil.rmtree(old)
            elif old.exists():
                old.unlink()
    descriptor.write_text(json.dumps({"protocol":1,"sourceHash":info["sourceHash"],"sha256":digest,
        "bytes":info["bytes"],"archiveBytes":archive.stat().st_size,"target":info["target"]}, indent=2))
    print(f"Staged task recall archive: {archive.stat().st_size} bytes")


def sign(bundle, identity):
    if platform.system() != "Darwin":
        return
    magics = {b"\xcf\xfa\xed\xfe",b"\xce\xfa\xed\xfe",b"\xca\xfe\xba\xbe",b"\xca\xfe\xba\xbf"}
    with tempfile.TemporaryDirectory() as temporary:
        entitlements = Path(temporary) / "python.plist"
        entitlements.write_bytes(plistlib.dumps({"com.apple.security.cs.disable-library-validation":True}))
        for file in sorted(bundle.rglob("*")):
            if not file.is_file():
                continue
            with file.open("rb") as stream:
                native = stream.read(4) in magics
            if native:
                args = ["codesign", "--force", "--sign", identity, "--options", "runtime", "--timestamp=none"]
                # Python loads separately signed extension modules. Development
                # identities have no Team ID; scope this exception to Python.
                if file.parent == bundle / "runtime/bin" and file.name.startswith("python"):
                    args += ["--entitlements", str(entitlements)]
                run(*args, str(file), stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--target")
    parser.add_argument("--stage", type=Path)
    args = parser.parse_args()
    machine = {"arm64":"aarch64","AMD64":"x86_64"}.get(platform.machine(), platform.machine())
    system = {"Darwin":"apple-darwin","Windows":"pc-windows-msvc","Linux":"unknown-linux-gnu"}[platform.system()]
    target = f"{machine}-{system}"
    if args.target and args.target != target:
        raise RuntimeError(f"Build the Python sidecar natively for {args.target}; current host is {target}")
    inputs = [ROOT / "uv.lock", ROOT / "pyproject.toml", ROOT / "main.py", Path(__file__)] + sorted((ROOT / "codepet_recall").glob("*.*"))
    identity = os.environ.get("APPLE_SIGNING_IDENTITY", "-")
    fingerprint = hashlib.sha256(b"".join(p.read_bytes() for p in inputs) + target.encode() + identity.encode()).hexdigest()
    output = args.output.resolve()
    manifest = output / "manifest.json"
    if manifest.exists() and json.loads(manifest.read_text()).get("sourceHash") == fingerprint:
        print(f"Task recall bundle is current: {output}")
        stage(output, args.stage)
        return
    run("uv", "python", "install", "3.12.14")
    python = Path(subprocess.check_output(["uv","python","find","--managed-python","3.12.14"], text=True).strip())
    prefix = Path(subprocess.check_output([str(python),"-I","-c","import sys; print(sys.base_prefix)"], text=True).strip())
    output.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="task-recall-build-", dir=output.parent) as temporary:
        bundle = Path(temporary) / "bundle"
        bundle.mkdir()
        shutil.copytree(prefix, bundle / "runtime", symlinks=False, ignore=shutil.ignore_patterns("__pycache__", "*.pyc"))
        requirements = Path(temporary) / "requirements.txt"
        run("uv", "export", "--project", str(ROOT), "--frozen", "--no-dev", "--no-emit-project", "--output-file", str(requirements), stdout=subprocess.DEVNULL)
        run("uv", "pip", "install", "--python", str(python), "--target", str(bundle / "packages"),
            "--require-hashes", "--only-binary", ":all:", "--no-binary", "langdetect", "-r", str(requirements))
        patch = patch_cognee(bundle / "packages")
        (bundle / "cognee-patches.json").write_text(json.dumps({"version":"1.6.10","patches":[patch]},indent=2))
        shutil.copy2(ROOT / "main.py", bundle / "main.py")
        shutil.copytree(ROOT / "codepet_recall", bundle / "codepet_recall", ignore=shutil.ignore_patterns("__pycache__"))
        shutil.copy2(ROOT / "uv.lock", bundle / "uv.lock")
        # Tokenizer vocabulary is data, not a model. Cache it at build time.
        env = {**os.environ, "TIKTOKEN_CACHE_DIR":str(bundle / "tokenizer-cache"), "PYTHONPATH":str(bundle / "packages")}
        run(str(python), "-c", "import tiktoken; tiktoken.get_encoding('cl100k_base'); tiktoken.get_encoding('o200k_base')", env=env)
        for cache in list(bundle.rglob("__pycache__")):
            shutil.rmtree(cache)
        sign(bundle, identity)
        files = [{"path":p.relative_to(bundle).as_posix(),"bytes":p.stat().st_size,
                  "sha256":hashlib.sha256(p.read_bytes()).hexdigest()} for p in sorted(bundle.rglob("*")) if p.is_file()]
        info = {"protocol":1,"target":target,"python":"3.12.14","cognee":"1.6.10","sourceHash":fingerprint,
                "pythonPath":"runtime/python.exe" if os.name == "nt" else "runtime/bin/python3.12",
                "bytes":sum(f["bytes"] for f in files),"files":files}
        (bundle / "manifest.json").write_text(json.dumps(info, indent=2))
        if output.exists():
            shutil.rmtree(output)
        shutil.move(str(bundle), output)
    print(json.dumps({k:v for k,v in info.items() if k != "files"}))
    stage(output, args.stage)


if __name__ == "__main__":
    main()
