import tempfile
import unittest
from pathlib import Path

from deploy_diagnostics import backup_targets


class BackupTests(unittest.TestCase):
    def test_backups_do_not_enter_service_include_directories(self):
        with tempfile.TemporaryDirectory() as root:
            root = Path(root)
            includes = root / 'etc/logrotate.d'
            includes.mkdir(parents=True)
            original = includes / 'coturn'
            original.write_text('/var/log/coturn/turnserver.log {}\n')
            backups = backup_targets([original], root / 'backups/deployment')
            self.assertEqual(list(includes.iterdir()), [original])
            self.assertEqual(backups[original].read_bytes(), original.read_bytes())
            # Refuse to overwrite a previous deployment's rollback snapshot.
            with self.assertRaises(FileExistsError):
                backup_targets([original], root / 'backups/deployment')


if __name__ == '__main__':
    unittest.main()
