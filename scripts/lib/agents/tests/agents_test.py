# Copyright 2026 The Fuchsia Authors. All rights reserved.
# Use of this source code is governed by a BSD-style license that can be
# found in the LICENSE file.

import unittest

import agents.agents as agents_lib


class TestAgents(unittest.TestCase):
    def test_get_agent_env_vars(self) -> None:
        vars_list = agents_lib.get_agent_env_vars()
        self.assertIn("ANTIGRAVITY_AGENT", vars_list)
        self.assertIn("GEMINI_CLI", vars_list)

    def test_is_invoked_by_agent(self) -> None:
        env: dict[str, str] = {}
        self.assertFalse(agents_lib.is_invoked_by_agent(env))

        env = {"ANTIGRAVITY_AGENT": "1"}
        self.assertTrue(agents_lib.is_invoked_by_agent(env))

        env = {"GEMINI_CLI": "1"}
        self.assertTrue(agents_lib.is_invoked_by_agent(env))


if __name__ == "__main__":
    unittest.main()
