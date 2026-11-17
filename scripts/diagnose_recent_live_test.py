import contextlib
import io
import unittest
from unittest.mock import patch

from diagnose_recent_live import discover_targets, host_candidates, main


PROCESSES = '''
1 0 /sbin/launchd
10 1 /Users/test/Downloads/CodePet.app/Contents/MacOS/code-pet
11 10 /bin/sh
12 11 /Users/test/Downloads/CodePet.app/Contents/Resources/provider-plugins/codex/codepet-provider-codex
13 12 /opt/homebrew/bin/codex
14 13 /some/native/helper
15 10 /some/codepet-provider-opencode
16 15 /opt/homebrew/bin/opencode
20 1 /private/var/folders/xyz/AppTranslocation/id/d/Code Pet.app/Contents/MacOS/code-pet
21 20 /some/codepet-provider-codex
22 21 /other/codex
30 1 /Applications/ChatGPT.app/Contents/MacOS/ChatGPT
31 30 /unrelated/codepet-provider-codex
32 31 /unrelated/codex
40 41 /cycle/codepet-provider-codex
41 40 /cycle/helper
'''


class ProcessSelectionTests(unittest.TestCase):
    def test_user_installation_and_app_translocation_are_candidates(self):
        self.assertEqual(set(host_candidates(PROCESSES)), {10, 20})

    def test_selected_host_includes_wrapped_provider_and_only_its_descendants(self):
        self.assertEqual(discover_targets(PROCESSES, 10), {
            10: 'Host', 12: 'Provider', 13: 'CodexNative', 14: 'CodexNative',
        })
        self.assertEqual(discover_targets(PROCESSES, 20), {
            20: 'Host', 21: 'Provider', 22: 'CodexNative',
        })

    def test_basename_only_and_renamed_bundle_are_supported(self):
        for command in ['code-pet', '/custom/我的工具.app/Contents/MacOS/code-pet',
                        '/custom/Code Pet.app/Contents/MacOS/Code Pet']:
            with self.subTest(command=command):
                self.assertEqual(host_candidates(f'10 1 {command}'), {10: command})

    def test_explicit_pid_handles_custom_executable_name(self):
        processes = '10 1 /custom/server\n11 10 /custom/codepet-provider-codex\n'
        self.assertEqual(discover_targets(processes, 10), {10: 'Host', 11: 'Provider'})
        self.assertEqual(discover_targets(processes, 999), {})

    def test_missing_or_malformed_rows_do_not_infer_a_host(self):
        self.assertEqual(discover_targets('bad line\nxx yy /custom/code-pet\n'), {})
        self.assertEqual(discover_targets('11 1 /unrelated/codepet-provider-codex'), {})

    def test_ambiguous_hosts_stop_before_sampling(self):
        output = io.StringIO()
        with patch('sys.argv', ['diagnose_recent_live.py']), \
                patch('platform.system', return_value='Darwin'), \
                patch('subprocess.check_output', return_value=PROCESSES) as ps, \
                patch('subprocess.run') as sample, contextlib.redirect_stderr(output):
            with self.assertRaises(SystemExit) as caught:
                main()
        self.assertEqual(caught.exception.code, 2)
        self.assertIn('--host-pid', output.getvalue())
        self.assertIn('-ww', ps.call_args.args[0])
        sample.assert_not_called()

    def test_list_is_read_only(self):
        output = io.StringIO()
        with patch('sys.argv', ['diagnose_recent_live.py', '--list']), \
                patch('platform.system', return_value='Darwin'), \
                patch('subprocess.check_output', return_value=PROCESSES), \
                patch('subprocess.run') as sample, contextlib.redirect_stdout(output):
            main()
        self.assertIn('PID=10', output.getvalue())
        self.assertIn('PID=20', output.getvalue())
        sample.assert_not_called()


if __name__ == '__main__':
    unittest.main()
