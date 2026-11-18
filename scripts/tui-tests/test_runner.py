"""Runner accounting regressions; no real tmux, engine, or credentials used."""
import os
import pathlib
import shlex
import shutil
import subprocess
import tempfile
import unittest

HERE = pathlib.Path(__file__).resolve().parent

# Each start appends one line: the working directory, HOME, whether that home's
# settings file is seeded, the variables that would select another home,
# whether a .coven-code sits in or above the working directory, and a variable
# only a caller's launch prefix sets.
ENGINE = r"""#!/bin/sh
seeded=no
grep -q hasCompletedOnboarding "$HOME/.coven-code/settings.json" 2>/dev/null && seeded=yes
above=no
[ -e /.coven-code ] && above=yes
dir="$(pwd -P)"
while [ -n "$dir" ]; do
  [ -e "$dir/.coven-code" ] && above=yes
  dir="${dir%/*}"
done
printf '%s\t%s\t%s\t%s\t%s\t%s\n' "$(pwd -P)" "$HOME" "$seeded" \
  "${COVEN_HOME-}${COVEN_CODE_HOME-}${COVEN_PARENT-}" "$above" "${CALLER_MARK-}" >>STARTS
exit 0
"""

# Shows a ready shell and a booted screen, and stands in for the pane's shell:
# a typed line that names the engine is run. Like a shell whose startup files
# changed directory and set the home-selecting variables, it runs that line
# from the caller's home with all three set.
TMUX = r"""#!/bin/sh
case "$*" in
  -V) printf 'tmux fixture\n' ;;
  *capture-pane*) printf '__SHELL_READY__ Coven v\n' ;;
  *send-keys*-l*ENGINE*)
    for typed; do :; done
    cd PANE_DIR && COVEN_HOME=PANE_DIR/.coven COVEN_CODE_HOME=PANE_DIR/.coven-code \
      COVEN_PARENT=coven sh -c "$typed" ;;
esac
exit 0
"""


class RunnerTests(unittest.TestCase):
    def make_suite(self, cases):
        # Exercise the actual runner and assertion library. Only the case inputs
        # and external executables are fixtures; never contact a real tmux server.
        # self.home stands in for the caller's home, and self.scratch holds the
        # tools and the temporary directory the runner may use. Both outlive the
        # run, so a test can read what it left.
        for name, prefix in (("home", "coven runner "), ("scratch", "coven-scratch-")):
            temp = tempfile.TemporaryDirectory(prefix=prefix)
            self.addCleanup(temp.cleanup)
            setattr(self, name, pathlib.Path(temp.name).resolve())
        self.suite = self.home / "scripts" / "tui-tests"
        (self.suite / "cases").mkdir(parents=True)
        for filename in ("run.sh", "lib.sh"):
            shutil.copy2(HERE / filename, self.suite / filename)
        for filename, content in cases.items():
            (self.suite / "cases" / filename).write_text(content, encoding="utf-8")
        # No space in the tools' path: tui_start types the engine's path unquoted.
        tools = self.scratch / "bin"
        tools.mkdir()
        self.starts = self.scratch / "starts"
        self.engine = tools / "coven-code"
        self.engine.write_text(ENGINE.replace("STARTS", shlex.quote(str(self.starts))), encoding="utf-8")
        self.engine.chmod(0o700)
        tmux = tools / "tmux"
        tmux.write_text(TMUX.replace("ENGINE", str(self.engine))
                        .replace("PANE_DIR", shlex.quote(str(self.home))), encoding="utf-8")
        tmux.chmod(0o700)
        # A space, so the scratch paths the runner types into the pane need quoting.
        self.tmp = self.scratch / "tmp dir"
        self.tmp.mkdir()

    def run_made_suite(self, *filters, cwd=None, env=None):
        bash = shutil.which("bash")
        if bash is None:
            self.fail("These harness regressions require bash")
        return subprocess.run(
            [bash, str(self.suite / "run.sh"), *filters],
            cwd=cwd or self.home,
            env={
                "PATH": str(self.engine.parent) + os.pathsep + os.defpath,
                "HOME": str(self.home),
                "TMPDIR": str(self.tmp),
                "COVEN_BIN": str(self.engine),
                "REQUIRE_TMUX": "1",
                **(env or {}),
            },
            capture_output=True, text=True, encoding="utf-8", timeout=20,
        )

    def run_suite(self, cases, *filters):
        self.make_suite(cases)
        return self.run_made_suite(*filters)

    def engine_starts(self):
        if not self.starts.exists():
            return []
        return [dict(zip(("cwd", "home", "seeded", "other_homes", "settings_above", "caller_mark"),
                         line.split("\t")))
                for line in self.starts.read_text(encoding="utf-8").splitlines()]

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

    # A headless start and a tmux start, as cases 01-07 make them: with no home
    # or working directory of their own.
    BARE_STARTS = {
        "01_bare.sh": 'register_case tc_bare\n'
        'tc_bare() { run_bin --version; tui_start; tui_stop; _pass "started twice"; }\n',
    }

    def test_bare_cases_start_away_from_the_callers_home_and_settings(self):
        # The caller's home holds personal settings, the run starts in a checkout
        # under it, and the caller's environment selects that home three ways.
        self.make_suite(self.BARE_STARTS)
        personal = self.home / ".coven-code"
        personal.mkdir()
        (personal / "settings.json").write_text('{"showSplash": false}', encoding="utf-8")
        checkout = self.home / "checkout"
        checkout.mkdir()
        result = self.run_made_suite(cwd=checkout, env={
            "COVEN_HOME": str(self.home / ".coven"), "COVEN_CODE_HOME": str(personal),
            "COVEN_PARENT": "coven",
        })
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        starts = self.engine_starts()
        self.assertEqual(len(starts), 2, f"want one headless and one tmux start: {starts}")
        for start in starts:
            cwd, home = pathlib.Path(start["cwd"]), pathlib.Path(start["home"])
            self.assertNotIn(self.home, (cwd, *cwd.parents), f"started in the caller's home: {start}")
            self.assertNotIn(self.home, (home, *home.parents), f"given the caller's home: {start}")
            self.assertEqual(start["settings_above"], "no", start)
            self.assertEqual(start["seeded"], "yes", start)
            self.assertEqual(start["other_homes"], "", start)
            self.assertNotIn(home, (cwd, *cwd.parents), f"started inside its own home: {start}")
        self.assertEqual(starts[0]["cwd"], starts[1]["cwd"], "the two starts used different directories")
        self.assertEqual(list(self.tmp.iterdir()), [], "the scratch directory was left behind")

    def test_tmpdir_for_the_cases_is_inside_the_checked_directory(self):
        # The other cases make their own scratch directories under TMPDIR.
        self.make_suite({
            "01_own.sh": 'register_case tc_own\n'
            'tc_own() { cd "$(mktemp -d "$TMPDIR/own.XXXXXX")" && run_bin --version; _pass "started"; }\n',
        })
        result = self.run_made_suite()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        (start,) = self.engine_starts()
        made = pathlib.Path(start["cwd"])
        self.assertEqual((made.parents[2], made.parent.name), (self.tmp, "tmp"), start)
        self.assertEqual(start["settings_above"], "no", start)

    def test_a_callers_launch_prefix_is_kept_inside_the_isolation(self):
        self.make_suite(self.BARE_STARTS)
        result = self.run_made_suite(env={"TUI_LAUNCH_PREFIX": "env CALLER_MARK=kept "})
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        typed = self.engine_starts()[1]
        self.assertEqual(typed["caller_mark"], "kept", typed)
        self.assertNotIn(self.home, pathlib.Path(typed["cwd"]).parents, typed)
        self.assertEqual((typed["seeded"], typed["other_homes"]), ("yes", ""), typed)

    def test_an_empty_or_root_home_is_not_taken_for_the_scratch_directory(self):
        for home in ("", "/"):
            with self.subTest(home=home):
                self.make_suite(self.BARE_STARTS)
                result = self.run_made_suite(env={"HOME": home})
                self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
                self.assertEqual(len(self.engine_starts()), 2)

    def assert_refused(self, result, reason):
        self.assertEqual(result.returncode, 2, result.stdout + result.stderr)
        self.assertIn("Refusing to run", result.stderr)
        self.assertIn(reason, result.stderr)
        self.assertEqual(self.engine_starts(), [])

    def test_refuses_a_scratch_directory_in_the_callers_home(self):
        self.make_suite(self.BARE_STARTS)
        unsafe = self.home / "tmp"
        unsafe.mkdir()
        self.assert_refused(self.run_made_suite(env={"TMPDIR": str(unsafe)}),
                            f"inside the home directory {self.home}")
        self.assertEqual(list(unsafe.iterdir()), [], "the refused scratch directory was left behind")

    def test_refuses_a_scratch_directory_that_a_symlink_puts_in_the_callers_home(self):
        self.make_suite(self.BARE_STARTS)
        (self.home / "tmp").mkdir()
        link = self.scratch / "link"
        link.symlink_to(self.home / "tmp")
        self.assert_refused(self.run_made_suite(env={"TMPDIR": str(link)}),
                            f"inside the home directory {self.home}")

    def test_refuses_a_scratch_directory_in_the_login_home_when_home_is_a_throwaway(self):
        # Only a directory in the real login home can show this, so use one in
        # this checkout. HOME is the fixture's home, as for every run here.
        import pwd  # Unix only, like the bash these tests need
        login = pathlib.Path(pwd.getpwuid(os.getuid()).pw_dir).resolve()
        if login == pathlib.Path(login.anchor) or login not in HERE.parents:
            self.skipTest("this checkout is not inside the login home")
        self.make_suite(self.BARE_STARTS)
        inside = tempfile.TemporaryDirectory(prefix="isolation-test-", dir=HERE)
        self.addCleanup(inside.cleanup)
        self.assert_refused(self.run_made_suite(env={"TMPDIR": inside.name}),
                            f"inside the home directory {login}")
        self.assertEqual(os.listdir(inside.name), [], "the refused scratch directory was left behind")

    def test_refuses_a_scratch_directory_below_project_settings(self):
        for below in ("", "tmp", "deeper/tmp"):
            with self.subTest(below=below):
                self.make_suite(self.BARE_STARTS)
                project = self.scratch / "project"
                (project / ".coven-code").mkdir(parents=True)
                (project / below).mkdir(parents=True, exist_ok=True)
                self.assert_refused(self.run_made_suite(env={"TMPDIR": str(project / below)}),
                                    f"{project / '.coven-code'} is above it")

    def test_relative_paths_from_the_caller_survive_the_move(self):
        # Paths given relative to the caller's directory must still resolve once
        # the run has left it.
        self.make_suite({
            "01_fails.sh": 'register_case tc_fails\n'
            'tc_fails() {\n'
            '  run_bin --version; assert_eq "$RUN_RC" 0 "ran"\n'
            '  assert_eq "$COVEN_PANIC_PROBE" "$WANT_ROOT/bin/probe" "probe path"\n'
            '  assert_eq "$(sh -c \'printf %s "$COVEN_TUI_PTY_RECEIPTS"\')" "$WANT_ROOT/receipts" "receipts path"\n'
            '  _fail "kept" "pane text"\n'
            '}\n',
        })
        result = self.run_made_suite(cwd=self.scratch, env={
            "COVEN_BIN": "bin/coven-code", "TUI_LOG_DIR": "logs", "TMPDIR": self.tmp.name,
            "COVEN_PANIC_PROBE": "bin/probe", "COVEN_TUI_PTY_RECEIPTS": "receipts",
            "WANT_ROOT": str(self.scratch),
        })
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertIn("3 passed  1 failed", result.stdout)
        self.assertEqual(len(self.engine_starts()), 1)
        self.assertIn("pane text", (self.scratch / "logs" / "tc_fails.log").read_text(encoding="utf-8"))
        self.assertEqual(list(self.tmp.iterdir()), [], "the scratch directory was left behind")


if __name__ == "__main__":
    unittest.main()
