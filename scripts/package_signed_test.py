#!/usr/bin/env python3

import sys
import unittest
from contextlib import redirect_stderr
from io import StringIO
from pathlib import Path
from unittest.mock import patch

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

    def test_uses_legacy_notarize_variable_names(self):
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

    def test_missing_credentials_stops_before_build_or_shell(self):
        with patch.dict(package_signed.os.environ, {}, clear=True), \
                patch.object(package_signed.platform, "system", return_value="Darwin"), \
                patch.object(package_signed.subprocess, "run") as run, \
                patch.object(package_signed, "remove_old_dmg_files") as remove, \
                redirect_stderr(StringIO()):
            with self.assertRaises(SystemExit) as error:
                package_signed.main()

        self.assertEqual(error.exception.code, 2)
        run.assert_not_called()
        remove.assert_not_called()

    def test_main_passes_explicit_environment_to_notarization(self):
        env = {"CODE_PET_NOTARY_KEYCHAIN_PROFILE": "code-pet-notary"}
        dmg = Path("test.dmg")
        with patch.dict(package_signed.os.environ, env, clear=True), \
                patch.object(package_signed.platform, "system", return_value="Darwin"), \
                patch.object(package_signed, "run") as run, \
                patch.object(package_signed, "remove_old_dmg_files"), \
                patch.object(package_signed, "find_created_dmg", return_value=dmg), \
                patch.object(package_signed, "notarize_dmg") as notarize:
            self.assertEqual(package_signed.main(), 0)

        notarize.assert_called_once_with(dmg, env)
        self.assertEqual(run.call_args_list[0].args[0],
                         ["npm", "run", "tauri", "build", "--", "--bundles", "dmg"])


if __name__ == "__main__":
    unittest.main()
