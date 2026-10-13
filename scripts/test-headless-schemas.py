# /// script
# requires-python = ">=3.10"
# dependencies = ["jsonschema==4.25.1"]
# ///
"""Validate the runtime v3 fixtures and their cross-field constraints.

Run from any directory: uv run scripts/test-headless-schemas.py
"""
import copy
import json
from pathlib import Path
import unittest

from jsonschema import Draft202012Validator

FIXTURES = Path(__file__).resolve().parents[1] / "src-rust/crates/cli/tests/headless_contract"


class HeadlessSchemas(unittest.TestCase):
    def setUp(self):
        self.brief = json.loads((FIXTURES / "session-brief.example.json").read_text())
        self.result = json.loads((FIXTURES / "result.example.json").read_text())
        self.validators = {}
        for name in ("session-brief", "result"):
            schema = json.loads((FIXTURES / f"{name}.schema.json").read_text())
            Draft202012Validator.check_schema(schema)
            self.validators[name] = Draft202012Validator(schema)

    def test_golden_fixtures(self):
        self.validators["session-brief"].validate(self.brief)
        self.validators["result"].validate(self.result)

    def test_brief_requires_version(self):
        del self.brief["contract_version"]
        self.assertFalse(self.validators["session-brief"].is_valid(self.brief))

    def test_result_status_and_exit_reason(self):
        for status in ("success", "failure", "needs_input", "partial"):
            for reason in (None, "test_failure", "ambiguous_spec", "git_conflict", "infra_error"):
                with self.subTest(status=status, reason=reason):
                    value = copy.deepcopy(self.result)
                    value.update(status=status, exit_reason=reason)
                    expected = (
                        reason is None if status == "success"
                        else status == "partial" or reason is not None
                    )
                    self.assertEqual(self.validators["result"].is_valid(value), expected)
        for status in ("failure", "needs_input"):
            with self.subTest(status=status, reason="absent"):
                value = copy.deepcopy(self.result)
                value["status"] = status
                value.pop("exit_reason", None)
                self.assertFalse(self.validators["result"].is_valid(value))

    def test_review_mode_and_evidence(self):
        for mode in ("none", "pull_request", "review_comment"):
            for evidence in ("not_applicable", "complete", "partial", "missing"):
                for files in ([], ["src/example.rs"]):
                    with self.subTest(mode=mode, evidence=evidence, files=files):
                        value = copy.deepcopy(self.result)
                        value["review"].update(mode=mode, evidence_status=evidence, reviewed_files=files)
                        expected = (
                            evidence == "not_applicable" if mode == "none"
                            else evidence != "not_applicable"
                            and (bool(files) or evidence == "missing")
                        )
                        self.assertEqual(self.validators["result"].is_valid(value), expected)


if __name__ == "__main__":
    unittest.main()
