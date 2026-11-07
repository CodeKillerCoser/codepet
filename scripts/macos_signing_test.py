#!/usr/bin/env python3

import base64
from contextlib import redirect_stdout
from io import StringIO
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

import macos_signing as signing


class SigningTest(unittest.TestCase):
    def environment(self, temporary):
        return {
            "GITHUB_ACTIONS": "true", "RUNNER_ENVIRONMENT": "github-hosted", "RUNNER_TEMP": temporary,
            "CODEPET_MACOS_CERTIFICATE_BASE64": base64.b64encode(b"fixture").decode(),
            "CODEPET_MACOS_CERTIFICATE_PASSWORD": "private-fixture-password",
            "CODEPET_MACOS_CERTIFICATE_SHA1": "A" * 40,
            "APPLE_SIGNING_IDENTITY": "-", "APPLE_PASSWORD": "unrelated-account",
            "CODEPET_PROVIDER_TARGET": "aarch64-apple-darwin",
        }

    def test_invalid_credentials_fail_before_keychain_changes(self):
        with patch.object(signing, "run") as run:
            for suffix, value in (("CERTIFICATE_BASE64", "!"), ("CERTIFICATE_PASSWORD", ""), ("CERTIFICATE_SHA1", "-")):
                env = self.environment("/tmp")
                env["CODEPET_MACOS_" + suffix] = value
                with self.assertRaises(ValueError), signing.signing_environment(env):
                    self.fail("Invalid credentials were accepted")
            run.assert_not_called()

    def test_developer_machine_rejected_before_trust_changes(self):
        with patch.object(signing, "run") as run:
            env = self.environment("/tmp")
            env["RUNNER_ENVIRONMENT"] = "self-hosted"
            with self.assertRaises(RuntimeError), signing.signing_environment(env):
                self.fail("Local signing setup was allowed")
            run.assert_not_called()

    def test_fingerprint_mismatch_stops_before_import(self):
        with tempfile.TemporaryDirectory() as directory, \
                patch.object(signing.sys, "platform", "darwin"), \
                patch.object(signing, "fingerprint", return_value="B" * 40), \
                patch.object(signing, "run", return_value=b"") as run:
            with self.assertRaises(ValueError), signing.signing_environment(self.environment(directory)):
                self.fail("Mismatched identity was accepted")
            self.assertFalse(any(call.args[0][0] == "security" for call in run.call_args_list))

    def test_build_failure_restores_keychains_deletes_private_keys_and_hides_credentials(self):
        def execute(args, **kwargs):
            if args == ["security", "list-keychains", "-d", "user"]:
                return b'"/tmp/original keychain-db"\n'
            return b""

        with tempfile.TemporaryDirectory() as directory, \
                patch.object(signing.sys, "platform", "darwin"), \
                patch.object(signing, "fingerprint", return_value="A" * 40), \
                patch.object(signing, "run", side_effect=execute) as run, \
                redirect_stdout(StringIO()):
            with self.assertRaisesRegex(RuntimeError, "build failed"):
                with signing.signing_environment(self.environment(directory)) as (env, identity):
                    self.assertEqual(env["APPLE_SIGNING_IDENTITY"], identity)
                    self.assertNotIn("APPLE_PASSWORD", env)
                    self.assertFalse(any(key.startswith("CODEPET_MACOS_") for key in env))
                    self.assertEqual(env["CODEPET_PROVIDER_TARGET"], "aarch64-apple-darwin")
                    raise RuntimeError("build failed")
            commands = [call.args[0] for call in run.call_args_list]
            self.assertIn(["security", "list-keychains", "-d", "user", "-s", "/tmp/original keychain-db"], commands)
            self.assertFalse(any("remove-trusted-cert" in command for command in commands))
            self.assertTrue(any("delete-keychain" in command for command in commands))
            self.assertEqual(list(Path(directory).iterdir()), [])

    def test_cleanup_attempts_all_actions_if_one_fails(self):
        def execute(args, **kwargs):
            if args == ["security", "list-keychains", "-d", "user", "-s"]:
                raise RuntimeError("failure")
            return b""

        with tempfile.TemporaryDirectory() as directory, \
                patch.object(signing.sys, "platform", "darwin"), \
                patch.object(signing, "fingerprint", return_value="A" * 40), \
                patch.object(signing, "run", side_effect=execute) as run, \
                redirect_stdout(StringIO()):
            with self.assertRaisesRegex(RuntimeError, "cleanup failed"):
                with signing.signing_environment(self.environment(directory)):
                    pass
            self.assertTrue(any("delete-keychain" in call.args[0] for call in run.call_args_list))

    def test_verification_extracts_the_leaf_and_rejects_hash_only_requirements(self):
        certificate = b"fixture DER certificate"
        identity = signing.hashlib.sha1(certificate).hexdigest().upper()
        requirement = 'designated => identifier "com.codepet.desktop" and anchor H"123"'
        def execute(args, **kwargs):
            for argument in args:
                if argument.startswith("--extract-certificates="):
                    Path(argument.split("=", 1)[1] + "0").write_bytes(certificate)
            if "-r-" in args:
                return requirement.encode()
            return b""
        with patch.object(signing, "run", side_effect=execute), redirect_stdout(StringIO()):
            self.assertEqual(signing.verify(Path("fixture.app"), identity, "com.codepet.desktop"), requirement)
            with self.assertRaisesRegex(ValueError, "pinned certificate"):
                signing.verify(Path("fixture.app"), "B" * 40, "com.codepet.desktop")
            requirement = 'designated => cdhash H"123"'
            with self.assertRaisesRegex(ValueError, "stable certificate"):
                signing.verify(Path("fixture.app"), identity, "com.codepet.desktop")

    def test_command_failure_does_not_print_password(self):
        with patch.object(signing.subprocess, "run", return_value=subprocess.CompletedProcess([], 1, b"secret", b"secret")):
            with self.assertRaises(RuntimeError) as error:
                signing.run(["security", "import", "file.p12", "-P", "secret"])
            self.assertNotIn("secret", str(error.exception))

    def test_command_timeout_does_not_expose_secret_arguments(self):
        with patch.object(signing.subprocess, "run", side_effect=subprocess.TimeoutExpired(["security", "-P", "secret"], 120)):
            with self.assertRaisesRegex(RuntimeError, "120-second") as error:
                signing.run(["security", "import", "file.p12", "-P", "secret"])
            self.assertNotIn("secret", str(error.exception))

    def test_generated_archive_roundtrip_and_existing_identity_preserved(self):
        with tempfile.TemporaryDirectory() as temporary, redirect_stdout(StringIO()):
            directory = Path(temporary) / "identity"
            old_umask = os.umask(0o077)
            try:
                signing.generate(directory)
            finally:
                os.umask(old_umask)
            before = (directory / "identity.p12").read_bytes()
            self.assertEqual(directory.stat().st_mode & 0o777, 0o700)
            for path in directory.iterdir():
                self.assertEqual(path.stat().st_mode & 0o777, 0o600)
            self.assertEqual((directory / "fingerprint.txt").read_text().strip(), signing.fingerprint(directory / "certificate.pem"))
            env = {**os.environ, "CODEPET_P12_PASSWORD": (directory / "password.txt").read_text()}
            signing.run(["openssl", "pkcs12", "-in", str(directory / "identity.p12"), "-passin", "env:CODEPET_P12_PASSWORD", "-noout"], env=env)
            with self.assertRaises(FileExistsError):
                signing.generate(directory)
            self.assertEqual(before, (directory / "identity.p12").read_bytes())

    def test_credentials_cannot_be_generated_inside_repository(self):
        with self.assertRaisesRegex(ValueError, "outside the repository"):
            signing.generate(signing.ROOT / "accidental-signing-credentials")


if __name__ == "__main__":
    unittest.main()
