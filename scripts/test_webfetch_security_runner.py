import importlib.util
from pathlib import Path
import unittest
import os
import sys
import tempfile

spec = importlib.util.spec_from_file_location("check", Path(__file__).with_name("check-webfetch-security.py"))
check = importlib.util.module_from_spec(spec)
spec.loader.exec_module(check)

class ReceiptTests(unittest.TestCase):
    @unittest.skipUnless(os.name == "posix", "POSIX process groups")
    def test_timeout_is_failure(self):
        with tempfile.TemporaryDirectory() as directory:
            result = check.run([sys.executable, "-c", "import time; time.sleep(10)"],
                               directory, os.environ.copy(), Path(directory)/"timeout.log", 0.1)
            self.assertEqual(result["returncode"], 124)
            self.assertFalse(check.qualified("", result["returncode"]))

    def test_requires_execution_not_cargo_success_alone(self):
        for text in ["", "test result: ok. 0 passed; 0 failed; 0 ignored;", "Finished test profile"]:
            self.assertFalse(check.qualified(text, 0))

    def test_rejects_failures_ignored_and_nonzero_exit(self):
        for text, code in [("test result: FAILED. 1 passed; 1 failed; 0 ignored;", 0),
                           ("test result: ok. 1 passed; 0 failed; 1 ignored;", 0),
                           ("test result: ok. 1 passed; 0 failed; 0 ignored;", 124)]:
            self.assertFalse(check.qualified(text, code))

    def test_all_reported_suites_must_pass(self):
        good = "test result: ok. 4 passed; 0 failed; 0 ignored;"
        self.assertTrue(check.qualified(good, 0))
        self.assertFalse(check.qualified(good + "\ntest result: FAILED. 0 passed; 1 failed; 0 ignored;", 0))

if __name__ == "__main__":
    unittest.main()
