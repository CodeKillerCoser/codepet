#!/usr/bin/env python3
"""Create a persistent development identity, or sign a release on a hosted runner."""

import argparse
import base64
from contextlib import contextmanager
import hashlib
import os
from pathlib import Path
import re
import secrets
import shlex
import subprocess
import sys
import tempfile
import time

ROOT = Path(__file__).resolve().parents[1]
APP = ROOT / "src-tauri/target/aarch64-apple-darwin/release/bundle/macos/Code Pet.app"
PREFIX = "CODEPET_MACOS_"


def run(args, *, env=None, include_stderr=False, public_errors=False):
    # Never include argv in errors: security takes passwords as command arguments.
    print(f"Signing helper: {Path(args[0]).name} {args[1]}", flush=True)
    try:
        result = subprocess.run(args, env=env, capture_output=True, timeout=120)
    except subprocess.TimeoutExpired:
        raise RuntimeError(f"{Path(args[0]).name} exceeded the 120-second signing command deadline") from None
    if result.returncode:
        if public_errors:
            print((result.stdout + result.stderr).decode(errors="replace"), file=sys.stderr, flush=True)
        detail = "see public verification output above" if public_errors else "command output suppressed to protect credentials"
        raise RuntimeError(f"{Path(args[0]).name} failed (exit {result.returncode}); {detail}")
    return result.stdout + result.stderr if include_stderr else result.stdout


def fingerprint(certificate):
    der = run(["openssl", "x509", "-in", str(certificate), "-outform", "DER"])
    # SHA-1 is macOS's identity selector; it is not the certificate signature algorithm.
    return hashlib.sha1(der).hexdigest().upper()


def generate(directory):
    if directory.resolve().is_relative_to(ROOT):
        raise ValueError("Store signing credentials outside the repository")
    directory.mkdir(mode=0o700, parents=True, exist_ok=False)
    password = secrets.token_urlsafe(48)
    (directory / "password.txt").write_text(password)
    env = {**os.environ, "CODEPET_P12_PASSWORD": password}
    with tempfile.TemporaryDirectory(dir=directory) as temporary:
        key = Path(temporary) / "key.pem"
        certificate = directory / "certificate.pem"
        run(["openssl", "req", "-new", "-x509", "-newkey", "rsa:3072", "-sha256",
             "-days", "3650", "-subj", "/CN=Code Pet Development Code Signing/O=Code Pet Development",
             "-addext", "basicConstraints=critical,CA:TRUE",
             "-addext", "keyUsage=critical,digitalSignature,keyCertSign",
             "-addext", "extendedKeyUsage=critical,codeSigning",
             "-passout", "env:CODEPET_P12_PASSWORD", "-keyout", str(key),
             "-out", str(certificate)], env=env)
        # macOS security import supports this PKCS#12 encoding, including older runners.
        run(["openssl", "pkcs12", "-export", "-inkey", str(key), "-in", str(certificate),
             "-name", "Code Pet Development Code Signing",
             "-passin", "env:CODEPET_P12_PASSWORD", "-passout", "env:CODEPET_P12_PASSWORD",
             "-keypbe", "PBE-SHA1-3DES", "-certpbe", "PBE-SHA1-3DES", "-macalg", "sha1",
             "-out", str(directory / "identity.p12")], env=env)
    identity = fingerprint(certificate)
    (directory / "fingerprint.txt").write_text(identity + "\n")
    print(f"Created identity {identity} in {directory}. Back up this directory; do not regenerate per build.")


def credentials(env):
    names = [PREFIX + suffix for suffix in ("CERTIFICATE_BASE64", "CERTIFICATE_PASSWORD", "CERTIFICATE_SHA1")]
    if any(not env.get(name) for name in names):
        raise ValueError("Required GitHub Secrets: " + ", ".join(names))
    identity = env[names[2]].strip().upper()
    if not re.fullmatch(r"[0-9A-F]{40}", identity):
        raise ValueError("CODEPET_MACOS_CERTIFICATE_SHA1 must be a 40-character certificate fingerprint")
    try:
        archive = base64.b64decode(env[names[0]], validate=True)
    except ValueError:
        raise ValueError("CODEPET_MACOS_CERTIFICATE_BASE64 is invalid") from None
    if not archive:
        raise ValueError("Certificate archive is empty")
    return archive, env[names[1]], identity


@contextmanager
def signing_environment(source):
    archive, password, identity = credentials(source)
    # Trust changes belong only on disposable GitHub-hosted runners, never a developer's Mac.
    if sys.platform != "darwin" or source.get("GITHUB_ACTIONS") != "true" or source.get("RUNNER_ENVIRONMENT") != "github-hosted":
        raise RuntimeError("Signing setup requires a disposable GitHub-hosted macOS runner")
    with tempfile.TemporaryDirectory(prefix="codepet-signing-", dir=source["RUNNER_TEMP"]) as temporary:
        directory = Path(temporary)
        p12 = directory / "identity.p12"
        certificate = directory / "certificate.pem"
        keychain = str(directory / "signing.keychain-db")
        p12.write_bytes(archive)
        env = {k: v for k, v in source.items() if not k.startswith(PREFIX)}
        certificate_env = {**env, "CODEPET_P12_PASSWORD": password}
        run(["openssl", "pkcs12", "-in", str(p12), "-clcerts", "-nokeys",
             "-passin", "env:CODEPET_P12_PASSWORD", "-out", str(certificate)], env=certificate_env)
        if fingerprint(certificate) != identity:
            raise ValueError("Certificate fingerprint does not match the pinned identity")
        run(["openssl", "x509", "-in", str(certificate), "-checkend", "2592000", "-noout"])
        keychain_password = secrets.token_urlsafe(48)
        previous = shlex.split(run(["security", "list-keychains", "-d", "user"]).decode())
        created = False
        try:
            run(["security", "create-keychain", "-p", keychain_password, keychain])
            created = True
            run(["security", "set-keychain-settings", "-lut", "21600", keychain])
            run(["security", "unlock-keychain", "-p", keychain_password, keychain])
            run(["security", "import", str(p12), "-k", keychain, "-P", password, "-T", "/usr/bin/codesign"])
            run(["security", "set-key-partition-list", "-S", "apple-tool:,apple:,codesign:", "-s", "-k", keychain_password, keychain])
            # Hosted macOS runners provide passwordless sudo. Admin-domain trust avoids a GUI prompt.
            run(["sudo", "-n", "security", "add-trusted-cert", "-d", "-r", "trustRoot", "-p", "codeSign", "-k", keychain, str(certificate)])
            run(["security", "list-keychains", "-d", "user", "-s", keychain, *previous])
            # Force selection by fingerprint; do not let inherited credentials select another identity.
            env = {k: v for k, v in env.items() if not k.startswith("APPLE_")}
            env["APPLE_SIGNING_IDENTITY"] = identity
            print(f"Using fixed Code Pet signing identity {identity}", flush=True)
            yield env, identity
        finally:
            original_error = sys.exc_info()[1]
            cleanup = [["security", "list-keychains", "-d", "user", "-s", *previous]]
            # remove-trusted-cert can wait for an authorization UI even under sudo.
            # This codeSign-only trust record is discarded with the hosted VM;
            # remove the private keychain here, without opening another authorization flow.
            if created:
                cleanup.append(["security", "delete-keychain", keychain])
            failed = False
            for command in cleanup:
                try:
                    run(command)
                except RuntimeError:
                    failed = True
            if failed:
                if original_error is None:
                    raise RuntimeError("Signing keychain cleanup failed; discard the hosted runner")
                print("Signing keychain cleanup also failed; discard the hosted runner", file=sys.stderr)


def verify(app, identity, identifier):
    # Verification arguments/output contain only public bundle and signature metadata.
    run(["codesign", "--verify", "--deep", "--strict", "--verbose=4", str(app)], public_errors=True)
    with tempfile.TemporaryDirectory(prefix="codepet-certificate-check-") as temporary:
        prefix = str(Path(temporary) / "cert")
        run(["codesign", "-d", f"--extract-certificates={prefix}", str(app)], public_errors=True)
        if hashlib.sha1(Path(prefix + "0").read_bytes()).hexdigest().upper() != identity:
            raise ValueError("Built app is not signed with the pinned certificate")
    details = run(["codesign", "-d", "-r-", str(app)], include_stderr=True, public_errors=True).decode()
    requirement = next((line for line in details.splitlines() if line.startswith("designated => ")), "")
    print(requirement or "No designated requirement returned by codesign", flush=True)
    # codesign normally expresses a self-signed identity as `certificate leaf`,
    # not `anchor`. Both refer to the same certificate in this one-cert chain.
    certificate_constraint = rf'(?:anchor|certificate (?:leaf|root|0|-1))\s*=\s*H"{identity}"'
    if (not requirement or "cdhash" in requirement or " or " in requirement
            or f'identifier "{identifier}"' not in requirement
            or not re.search(certificate_constraint, requirement, re.IGNORECASE)):
        raise ValueError("Built app lacks a stable certificate-based designated requirement")
    return requirement


def smoke(env, identity):
    with tempfile.TemporaryDirectory(prefix="codepet-signing-smoke-") as temporary:
        directory = Path(temporary)
        requirements = []
        binaries = []
        for version in (1, 2):
            source = directory / f"v{version}.c"
            binary = directory / f"v{version}"
            source.write_text(f"int main(void) {{ return {version}; }}\n")
            run(["xcrun", "clang", str(source), "-o", str(binary)])
            run(["codesign", "--force", "--sign", identity, "--timestamp=none", "--identifier", "com.codepet.signing-probe", str(binary)], env=env)
            requirements.append(verify(binary, identity, "com.codepet.signing-probe"))
            binaries.append(binary.read_bytes())
        if binaries[0] == binaries[1] or requirements[0] != requirements[1]:
            raise ValueError("Different builds did not retain the same signing identity")
        print("Two different binaries have the same certificate-based designated requirement.", flush=True)


def build():
    with signing_environment(os.environ) as (env, identity):
        smoke(env, identity)
        # Tauri deletes the intermediate .app when only "dmg" is requested.
        # Keep it for signature verification; distribution still uploads only the DMG.
        command = ["npm", "run", "tauri", "--", "build", "--target", "aarch64-apple-darwin", "--bundles", "app,dmg", "--config", "src-tauri/tauri.macos-dmg.conf.json"]
        for attempt in range(1, 4):
            result = subprocess.run(command, cwd=ROOT, env=env)
            if result.returncode == 0:
                break
            if attempt == 3:
                raise RuntimeError("Tauri build failed after three attempts")
            time.sleep(attempt * 20)
        verify(APP, identity, "com.codepet.desktop")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    generator = commands.add_parser("generate")
    generator.add_argument("--output", type=Path, required=True)
    commands.add_parser("build")
    args = parser.parse_args()
    os.umask(0o077)
    try:
        if args.command == "generate":
            generate(args.output.expanduser().resolve())
        else:
            build()
    except (ValueError, RuntimeError, OSError) as error:
        print(str(error), file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
