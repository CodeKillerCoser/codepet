#!/usr/bin/env python3

import sys
import unittest
from contextlib import redirect_stderr
from io import StringIO
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "scripts"))

import package_signed


class NotarizationArgsTest(unittest.TestCase):
    def test_prefers_keychain_profile(self):
        args = package_signed.notarization_args(
            {
                "CODE_PET_NOTARY_KEYCHAIN_PROFILE": "code-pet-notary",
                "APPLE_ID": "ignored@example.com",
                "APPLE_PASSWORD": "ignored",
                "APPLE_TEAM_ID": "IGNORED",
            }
        )

        self.assertEqual(args, ["--keychain-profile", "code-pet-notary"])

    def test_uses_apple_id_credentials(self):
        args = package_signed.notarization_args(
            {
                "APPLE_ID": "dev@example.com",
                "APPLE_APP_SPECIFIC_PASSWORD": "app-password",
                "APPLE_TEAM_ID": "TEAM123",
            }
        )

        self.assertEqual(
            args,
            [
                "--apple-id",
                "dev@example.com",
                "--password",
                "app-password",
                "--team-id",
                "TEAM123",
            ],
        )

    def test_uses_existing_zshrc_notarize_variable_names(self):
        args = package_signed.notarization_args(
            {
                "APPLE_NOTARIZE_APPLE_ID": "dev@example.com",
                "APPLE_NOTARIZE_PWD": "app-password",
                "APPLE_NOTARIZE_TEAM_ID": "TEAM123",
            }
        )

        self.assertEqual(
            args,
            [
                "--apple-id",
                "dev@example.com",
                "--password",
                "app-password",
                "--team-id",
                "TEAM123",
            ],
        )

    def test_fails_without_credentials(self):
        with redirect_stderr(StringIO()):
            with self.assertRaises(SystemExit) as error:
                package_signed.notarization_args({})

        self.assertEqual(error.exception.code, 2)

    def test_shell_fallback_imports_only_notary_environment(self):
        class Result:
            returncode = 0
            stdout = "\n".join(
                [
                    "APPLE_NOTARIZE_APPLE_ID=dev@example.com",
                    "APPLE_NOTARIZE_PWD=app-password",
                    "APPLE_NOTARIZE_TEAM_ID=TEAM123",
                    "UNRELATED_SECRET=ignored",
                ]
            )

        original_run = package_signed.subprocess.run
        package_signed.subprocess.run = lambda *args, **kwargs: Result()
        try:
            env = package_signed.notary_env_with_shell_fallback({})
        finally:
            package_signed.subprocess.run = original_run

        self.assertEqual(env["APPLE_NOTARIZE_APPLE_ID"], "dev@example.com")
        self.assertEqual(env["APPLE_NOTARIZE_PWD"], "app-password")
        self.assertEqual(env["APPLE_NOTARIZE_TEAM_ID"], "TEAM123")
        self.assertNotIn("UNRELATED_SECRET", env)


if __name__ == "__main__":
    unittest.main()
