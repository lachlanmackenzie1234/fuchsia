# Copyright 2026 The Fuchsia Authors. All rights reserved.
# Use of this source code is governed by a BSD-style license that can be
# found in the LICENSE file.

"""Analysis failure tests for `fx_test()` argument validation."""

load("//build/bazel/rules/testing:defs.bzl", "analysis_failure_test")
load("//build/bazel/rules/testing:fx_test.bzl", "fx_test")

_AEMU_ENV = {"dimensions": {"device_type": "AEMU"}}

# Each case is (name, extra `fx_test()` kwargs, expected error substring).
_CASES = [
    (
        "environments_not_a_list",
        {"environments": _AEMU_ENV},
        "Test 'environments' must be a list of environment dicts, got dict.",
    ),
    (
        "environments_empty",
        {"environments": []},
        "Test 'environments' must not be empty.",
    ),
    (
        "build_only_with_environments",
        {"build_only": True, "environments": [_AEMU_ENV]},
        "build_only tests should not specify environments",
    ),
    (
        "environment_not_a_dict",
        {"environments": ["AEMU"]},
        "Each entry in 'environments' must be a dict, got string.",
    ),
    (
        "unknown_environment_field",
        {"environments": [{"dimensions": {"device_type": "AEMU"}, "device_type": "AEMU"}]},
        "Unknown environment field 'device_type'",
    ),
    (
        "missing_dimensions",
        {"environments": [{"tags": ["emulated"]}]},
        "Each environment must specify a non-empty 'dimensions' dict.",
    ),
    (
        "tags_in_dimensions",
        {"environments": [{"dimensions": {"device_type": "AEMU", "tags": ["emulated"]}}]},
        "'tags' are only valid in an environment dict, not in 'dimensions'.",
    ),
    (
        "nested_dimensions",
        {"environments": [{"dimensions": _AEMU_ENV}]},
        "Found nested 'dimensions' field in environment dimensions.",
    ),
    (
        "emulator_not_a_dict",
        {"environments": [{"dimensions": {"device_type": "QEMU"}, "emulator": "1cpu"}]},
        "Environment 'emulator' field must be a dict, got string.",
    ),
    (
        "emulator_without_name",
        {"environments": [{"dimensions": {"device_type": "QEMU"}, "emulator": {"device": "x64-emu-min"}}]},
        "The 'emulator' dict requires a unique 'name'.",
    ),
    (
        "uefi_emulator_without_vbmeta",
        {"environments": [{"dimensions": {"device_type": "QEMU"}, "emulator": {"name": "uefi", "uefi": True}}]},
        "Emulator environments with 'uefi' set to True must provide 'vbmeta_key' and 'vbmeta_key_metadata'.",
    ),
    (
        "uefi_emulator_without_vbmeta_key_metadata",
        {"environments": [{"dimensions": {"device_type": "QEMU"}, "emulator": {"name": "uefi", "uefi": True, "vbmeta_key": "key.pem"}}]},
        "Emulator environments with 'uefi' set to True must provide 'vbmeta_key' and 'vbmeta_key_metadata'.",
    ),
]

def fx_test_validation_tests(name, package, **kwargs):
    """Defines an `analysis_failure_test()` for each invalid `fx_test()` call.

    Args:
        name: The name of the `test_suite()` grouping all the failure tests.
        package: An `fx_package()` target with test components, used by every
            `fx_test()` under test so that only the argument being tested is
            invalid.
        **kwargs: Common attributes for the `test_suite()` (e.g. `visibility`).
    """
    tests = []
    for case_name, fx_test_kwargs, expected_message in _CASES:
        target_name = "{}_{}".format(name, case_name)
        fx_test(
            name = target_name + "_fx_test",
            package = package,
            tags = ["manual"],
            **fx_test_kwargs
        )
        analysis_failure_test(
            name = target_name,
            target_under_test = ":{}_fx_test".format(target_name),
            expected_message = expected_message,
        )
        tests.append(":" + target_name)

    native.test_suite(
        name = name,
        tests = tests,
        **kwargs
    )
