# Copyright 2026 The Fuchsia Authors. All rights reserved.
# Use of this source code is governed by a BSD-style license that can be
# found in the LICENSE file.

"""Tests for the checks in //scripts/shac/gn.star."""

load("//scripts/shac/gn.star", "gn_no_print")

_MSG = "Avoid print() in GN files. It pollutes stdout and breaks automated tools (like gndoc). Consider using temporary prints and removing them before landing."

def test_gn_no_print():
    res = testing.run(gn_no_print, files = {
        "BUILD.gn": "group(\"foo\") {\n  print(\"a\")\n  print(\"b\")\n}\n",
        "foo.gni": "x = 1\n  print (x)\n",
    })
    asserts.eq(res.findings, (
        # Only the first print() in each file is reported.
        testing.finding(level = "warning", message = _MSG, filepath = "BUILD.gn", line = 2, col = 3),
        testing.finding(level = "warning", message = _MSG, filepath = "foo.gni", line = 2, col = 3),
    ))

def test_gn_no_print_comments():
    res = testing.run(gn_no_print, files = {
        "BUILD.gn": "  # print(\"a\")\ngroup(\"foo\") {}\n",
    })
    asserts.eq(res.findings, ())

def test_gn_no_print_other_files():
    res = testing.run(gn_no_print, files = {
        "foo.py": "print(\"a\")\n",
    })
    asserts.eq(res.findings, ())
