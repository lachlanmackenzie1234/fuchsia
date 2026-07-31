# Copyright 2022 The Fuchsia Authors. All rights reserved.
# Use of this source code is governed by a BSD-style license that can be
# found in the LICENSE file.

"""Utilities for extracting, creating, and manipulating debug symbols."""

load(
    "@fuchsia_rules_common//debug_symbols:debug_symbols.bzl",
    _fuchsia_unstripped_binary = "fuchsia_unstripped_binary",
)
load(
    "@fuchsia_rules_common//debug_symbols:providers.bzl",
    "FuchsiaDebugSymbolInfo",
)

def _fuchsia_debug_symbols_impl(ctx):
    return [
        FuchsiaDebugSymbolInfo(build_id_dirs_mapping = {
            ctx.file.source_search_root: depset(transitive = [
                target[DefaultInfo].files
                for target in ctx.attr.build_id_dirs
            ]),
        }),
    ]

fuchsia_debug_symbols = rule(
    doc = """Rule-based constructor for FuchsiaDebugSymbolInfo.""",
    implementation = _fuchsia_debug_symbols_impl,
    attrs = {
        "source_search_root": attr.label(
            doc = "A search root file or directory, used by zxdb to locate source files.",
            mandatory = True,
            allow_single_file = True,
        ),
        "build_id_dirs": attr.label_list(
            doc = "The build_id directories with symbols to be registered.",
            mandatory = True,
            allow_files = True,
        ),
    },
)

fuchsia_unstripped_binary = _fuchsia_unstripped_binary
