"""Runner accounting regressions; no real tmux, engine, or credentials used."""
import os
import pathlib
import shutil
import subprocess
import tempfile
import unittest

HERE = pathlib.Path(__file__).resolve().parent


class RunnerTests(unittest.TestCase):
    def run_suite(self, cases, *filters):
        # Exercise the actual runner and assertion library. Only the case inputs
        # and external executables are fixtures; never contact a real tmux server.
        with tempfile.TemporaryDirectory(prefix="coven runner ") as temp:
            root = pathlib.Path(temp)
            suite = root / "scripts" / "tui-tests"
            (suite / "cases").mkdir(parents=True)
            for filename in ("run.sh", "lib.sh"):
                shutil.copy2(HERE / filename, suite / filename)
            for filename, content in cases.items():
                (suite / "cases" / filename).write_text(content, encoding="utf-8")
            tools = root / "bin"
            tools.mkdir()
            tmux = tools / "tmux"
            tmux.write_text(
                '#!/bin/sh\nif [ "$1" = "-V" ]; then printf "tmux fixture\\n"; fi\n'
                'exit 0\n', encoding="utf-8",
            )
            tmux.chmod(0o700)
            engine = tools / "coven-code"
            engine.write_text("#!/bin/sh\nexit 0\n", encoding="utf-8")
            engine.chmod(0o700)
            bash = shutil.which("bash")
            if bash is None:
                self.fail("These harness regressions require bash")
            return subprocess.run(
                [bash, str(suite / "run.sh"), *filters],
                cwd=root,
                env={
                    "PATH": str(tools) + os.pathsep + os.defpath,
                    "HOME": str(root),
                    "COVEN_BIN": str(engine),
                    "REQUIRE_TMUX": "1",
                },
                capture_output=True, text=True, encoding="utf-8", timeout=5,
            )

    def test_successful_assertion_passes(self):
        result = self.run_suite({
            "01_ok.sh": 'register_case tc_ok\ntc_ok() { assert_eq yes yes "checked"; }\n',
        })
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("1 passed", result.stdout)

    def test_failed_assertion_fails(self):
        result = self.run_suite({
            "01_bad.sh": 'register_case tc_bad\ntc_bad() { assert_eq no yes "checked"; }\n',
        })
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertIn("1 failed", result.stdout)

    def test_nonzero_case_without_assertions_fails(self):
        result = self.run_suite({
            "01_error.sh": 'register_case tc_error\ntc_error() { return 17; }\n',
        })
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertIn("17", result.stdout)
        self.assertIn("tc_error", result.stdout)

    def test_nonzero_case_after_passing_assertion_fails(self):
        result = self.run_suite({
            "01_error.sh": 'register_case tc_error\ntc_error() { _pass "checked"; return 23; }\n',
        })
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertIn("23", result.stdout)

    def test_later_case_runs_after_nonzero_case(self):
        result = self.run_suite({
            "01_error.sh": 'register_case tc_error\ntc_error() { return 17; }\n',
            "02_ok.sh": 'register_case tc_ok\ntc_ok() { _pass "later case ran"; }\n',
        })
        self.assertIn("later case ran", result.stdout)
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)

    def test_empty_case_fails(self):
        result = self.run_suite({
            "01_empty.sh": 'register_case tc_empty\ntc_empty() { :; }\n',
        })
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertIn("no assertions", result.stdout)

    def test_empty_case_cannot_borrow_previous_assertions(self):
        result = self.run_suite({
            "01_ok.sh": 'register_case tc_ok\ntc_ok() { _pass "checked"; }\n',
            "02_empty.sh": 'register_case tc_empty\ntc_empty() { :; }\n',
        })
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertIn("tc_empty", result.stdout)

    def test_load_failure_fails_before_running_partial_catalog(self):
        result = self.run_suite({
            "01_bad.sh": 'register_case tc_partial\n'
            'tc_partial() { _pass "PARTIAL_CATALOG_EXECUTED"; }\nreturn 19\n',
        })
        self.assertEqual(result.returncode, 2, result.stdout + result.stderr)
        self.assertNotIn("PARTIAL_CATALOG_EXECUTED", result.stdout)
        self.assertIn("01_bad.sh", result.stderr)

    def test_undefined_registered_case_fails(self):
        result = self.run_suite({"01_bad.sh": 'register_case tc_missing\n'})
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertIn("tc_missing", result.stdout)

    def test_skipped_only_suite_is_not_success(self):
        result = self.run_suite({
            "01_skip.sh": 'register_case tc_skip\ntc_skip() { _skip "unavailable"; }\n',
        })
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertIn("1 skipped", result.stdout)

    def test_explicit_skip_with_real_assertion_retains_partial_mode(self):
        result = self.run_suite({
            "01_ok.sh": 'register_case tc_ok\ntc_ok() { _pass "headless"; }\n',
            "02_skip.sh": 'register_case tc_skip\ntc_skip() { _skip "interactive unavailable"; }\n',
        })
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("1 passed", result.stdout)
        self.assertIn("1 skipped", result.stdout)

    def test_failed_assertion_survives_later_pass(self):
        result = self.run_suite({
            "01_bad.sh": 'register_case tc_bad\ntc_bad() { _fail "first"; }\n',
            "02_ok.sh": 'register_case tc_ok\ntc_ok() { _pass "second"; }\n',
        })
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertIn("second", result.stdout)

    def test_unmatched_filter_is_configuration_error(self):
        result = self.run_suite({
            "01_ok.sh": 'register_case tc_ok\ntc_ok() { _pass "checked"; }\n',
        }, "not-a-case")
        self.assertEqual(result.returncode, 2, result.stdout + result.stderr)
        self.assertIn("No test cases matched", result.stderr)

    def test_help_remains_success_without_running_cases(self):
        result = self.run_suite({
            "01_bad.sh": 'register_case tc_bad\ntc_bad() { _fail "must not run"; }\n',
        }, "--help")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertNotIn("must not run", result.stdout)


if __name__ == "__main__":
    unittest.main()
