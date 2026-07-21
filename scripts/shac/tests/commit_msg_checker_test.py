#!/usr/bin/env fuchsia-vendored-python
# Copyright 2026 The Fuchsia Authors. All rights reserved.
# Use of this source code is governed by a BSD-style license that can be
# found in the LICENSE file.

import sys
import unittest
from pathlib import Path

# Add parent directory to path so we can import commit_msg_checker
sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

import commit_msg_checker


class TestCommitMessageChecker(unittest.TestCase):
    def test_valid_message(self) -> None:
        msg = (
            "Add commit message checker\n\n"
            "This change implements a commit message checker.\n\n"
            "Bug: 12345\n"
            "Test: Added unit tests.\n"
        )
        findings = commit_msg_checker.check_commit_message(msg)
        self.assertEqual(findings, [])

    def test_subject_line_too_long(self) -> None:
        long_subject = "A" * 70
        msg = f"{long_subject}\n\nBug: 1\nTest: 1"
        findings = commit_msg_checker.check_commit_message(msg)
        self.assertTrue(
            any(
                "Subject line exceeds 65 characters" in f["message"]
                for f in findings
            )
        )

    def test_body_line_too_long_ignores_urls(self) -> None:
        long_url = "https://fuchsia.dev/" + "a" * 80
        msg = f"Subject line\n\nSee {long_url} for details.\n\nBug: 1\nTest: 1"
        findings = commit_msg_checker.check_commit_message(msg)
        self.assertEqual(findings, [])

    def test_quoted_revert_long_line_exempt(self) -> None:
        msg = (
            'Revert "Subject"\n\n' "> " + "a" * 80 + "\n\n" "Bug: 1\n" "Test: 1"
        )
        findings = commit_msg_checker.check_commit_message(msg)
        self.assertEqual(findings, [])


if __name__ == "__main__":
    unittest.main()
