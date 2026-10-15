"""Polling boundary regressions; no tmux server or model credentials needed."""
import pathlib
import subprocess
import unittest

LIB = pathlib.Path(__file__).with_name("lib.sh")


class PollingTests(unittest.TestCase):
    def run_shell(self, body):
        return subprocess.run(
            ["bash", "-c", 'source "$1"; ' + body, "polling-test", str(LIB)],
            capture_output=True, text=True, timeout=5,
        )

    def test_capture_failure_is_not_absence(self):
        result = self.run_shell('tui_capture() { return 1; }; wait_absent marker 1')
        self.assertEqual(result.returncode, 2, result.stderr)
        self.assertIn("capture", result.stderr)

    def test_partial_failed_capture_is_not_presence(self):
        result = self.run_shell(
            'tui_capture() { printf "marker\\n"; return 1; }; wait_for marker 1'
        )
        self.assertEqual(result.returncode, 2, result.stderr)

    def test_large_capture_matches_without_sigpipe(self):
        result = self.run_shell('''
            tui_capture() {
                printf 'marker\\n'
                local i
                for ((i=0; i<10000; i++)); do printf 'padding padding padding\\n'; done
            }
            wait_for marker 1
        ''')
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_present_and_absent(self):
        for command in ('wait_for "a[b]" 1', 'wait_absent missing 1'):
            with self.subTest(command=command):
                result = self.run_shell('tui_capture() { printf "a[b]\\n"; }; ' + command)
                self.assertEqual(result.returncode, 0, result.stderr)

    def test_timeout(self):
        for command in ('wait_for missing 1', 'wait_absent marker 1'):
            with self.subTest(command=command):
                result = self.run_shell(
                    'tui_capture() { printf "marker\\n"; }; sleep() { :; }; ' + command
                )
                self.assertEqual(result.returncode, 1, result.stderr)


if __name__ == "__main__":
    unittest.main()
