#!/bin/bash
# Copyright 2020 The Fuchsia Authors. All rights reserved.
# Use of this source code is governed by a BSD-style license that can be
# found in the LICENSE file.
#
# Tests that femu is able to correctly interact with the fx emu command and
# its dependencies like fvm and aemu. These tests do not actually start up
# the emulator, but check the arguments are as expected.

set -e
SCRIPT_SRC_DIR="$(cd "$( dirname "${BASH_SOURCE[0]}" )" >/dev/null 2>&1 && pwd)"
# shellcheck disable=SC1090
source "${SCRIPT_SRC_DIR}/gn-bash-test-lib.sh"


# Specify a simulated CIPD instance id for aemu.version
AEMU_VERSION="git_revision:unknown"
AEMU_LABEL="$(echo "${AEMU_VERSION}" | tr ':/' '_')"
if is-mac; then
  AEMU_PLATFORM="mac-amd64"
else
  AEMU_PLATFORM="linux-amd64"
fi

# Verifies that the correct emulator command is run by femu when no arguments are provided on the command line.
TEST_femu_noargs() {

  PATH_DIR_FOR_TEST="${BT_TEMP_DIR}/_isolated_path_for"
  export PATH="${PATH_DIR_FOR_TEST}:${PATH}"

  # Create fake ZIP file download so femu.sh doesn't try to download it, and
  # later on provide a mocked emulator script so it doesn't try to unzip it.
  touch "${FUCHSIA_WORK_DIR}/emulator/aemu-${AEMU_PLATFORM}-${AEMU_LABEL}.zip"

  # Need to configure a DISPLAY so that we can get past the graphics error checks
  export DISPLAY="fakedisplay"

  # Run command.
  BT_EXPECT gn-test-run-bash-script "${BT_TEMP_DIR}/scripts/sdk/gn/base/bin/femu.sh"

  # Verify that fvm resized the disk file by 2x from the input 1024 to 2048.
  # shellcheck disable=SC1090
  source "${BT_TEMP_DIR}/scripts/sdk/gn/base/tools/fvm.mock_state"
  gn-test-check-mock-args _ANY_ _ANY_ extend --length 2048

  # Check that fpave.sh was called to download the needed system images
  # shellcheck disable=SC1090
  source "${BT_TEMP_DIR}/scripts/sdk/gn/base/bin/fpave.sh.mock_state"
  gn-test-check-mock-args _ANY_ --prepare --image qemu-x64 --bucket fuchsia --work-dir "${FUCHSIA_WORK_DIR}"

  # Check that fserve.sh was called to download the needed system images
  # shellcheck disable=SC1090
  source "${BT_TEMP_DIR}/scripts/sdk/gn/base/bin/fserve.sh.mock_state"
  gn-test-check-mock-args _ANY_ --prepare --image qemu-x64 --bucket fuchsia --work-dir "${FUCHSIA_WORK_DIR}"

  # Verify that zbi was called to add the authorized_keys
  # shellcheck disable=SC1090
  source "${BT_TEMP_DIR}/scripts/sdk/gn/base/tools/zbi.mock_state"
  gn-test-check-mock-args _ANY_ -o _ANY_ "${FUCHSIA_WORK_DIR}/image/zircon-a.zbi" --entry "data/ssh/authorized_keys=${FUCHSIA_WORK_DIR}/.ssh/authorized_keys"

  # Verify some of the arguments passed to the emulator binary
  # shellcheck disable=SC1090
  source "${FUCHSIA_WORK_DIR}/emulator/aemu-${AEMU_PLATFORM}-${AEMU_LABEL}/emulator.mock_state"
  gn-test-check-mock-partial -fuchsia
}

# Verifies that the correct emulator command is run by femu, along with the image setup.
# This tests the -N option for networking.
TEST_femu_networking() {

  PATH_DIR_FOR_TEST="${BT_TEMP_DIR}/_isolated_path_for"
  export PATH="${PATH_DIR_FOR_TEST}:${PATH}"

  # Create fake "ip tuntap show" command to let fx emu know the network is configured with some mocked output
  cat >"${PATH_DIR_FOR_TEST}/ip.mock_side_effects" <<INPUT
echo "qemu: tap persist user 238107"
INPUT

  # Create fake ZIP file download so femu.sh doesn't try to download it, and
  # later on provide a mocked emulator script so it doesn't try to unzip it.
  touch "${FUCHSIA_WORK_DIR}/emulator/aemu-${AEMU_PLATFORM}-${AEMU_LABEL}.zip"

  # Need to configure a DISPLAY so that we can get past the graphics error checks
  export DISPLAY="fakedisplay"

  # OSX may not have the tun/tap driver installed, and you cannot bypass the
  # network checks, so need to work around this for the test. Linux does not
  # need a fake network because we use a fake ip command.
  if is-mac && [[ ! -c /dev/tap0 ]]; then
    NETWORK_ARGS=( -N -I fakenetwork )
  else
    NETWORK_ARGS=( -N )
  fi

  # Run command.
  BT_EXPECT gn-test-run-bash-script "${BT_TEMP_DIR}/scripts/sdk/gn/base/bin/femu.sh" \
    "${NETWORK_ARGS[*]}" \
    --unknown-arg1-to-qemu \
    --authorized-keys "${BT_TEMP_DIR}/scripts/sdk/gn/base/testdata/authorized_keys" \
    --unknown-arg2-to-qemu

  # Verify that fvm resized the disk file by 2x from the input 1024 to 2048.
  # shellcheck disable=SC1090
  source "${BT_TEMP_DIR}/scripts/sdk/gn/base/tools/fvm.mock_state"
  gn-test-check-mock-args _ANY_ _ANY_ extend --length 2048

  # Check that fpave.sh was called to download the needed system images
  # shellcheck disable=SC1090
  source "${BT_TEMP_DIR}/scripts/sdk/gn/base/bin/fpave.sh.mock_state"
  gn-test-check-mock-args _ANY_ --prepare --image qemu-x64 --bucket fuchsia --work-dir "${FUCHSIA_WORK_DIR}"

  # Check that fserve.sh was called to download the needed system images
  # shellcheck disable=SC1090
  source "${BT_TEMP_DIR}/scripts/sdk/gn/base/bin/fserve.sh.mock_state"
  gn-test-check-mock-args _ANY_ --prepare --image qemu-x64 --bucket fuchsia --work-dir "${FUCHSIA_WORK_DIR}"

  # Verify that zbi was called to add the authorized_keys
  # shellcheck disable=SC1090
  source "${BT_TEMP_DIR}/scripts/sdk/gn/base/tools/zbi.mock_state"
  gn-test-check-mock-args _ANY_ -o _ANY_ "${FUCHSIA_WORK_DIR}/image/zircon-a.zbi" --entry "data/ssh/authorized_keys=${BT_TEMP_DIR}/scripts/sdk/gn/base/testdata/authorized_keys"

  # Verify some of the arguments passed to the emulator binary
  # shellcheck disable=SC1090
  source "${FUCHSIA_WORK_DIR}/emulator/aemu-${AEMU_PLATFORM}-${AEMU_LABEL}/emulator.mock_state"
  # The mac address is computed with a hash function in fx emu based on the device name.
  # We test the generated mac address since other scripts hard code this to SSH into the device.
  gn-test-check-mock-partial -fuchsia
  if is-mac; then
    if [[ -c /dev/tap0 ]]; then
      gn-test-check-mock-partial -netdev type=tap,ifname=tap0,id=net0,script="${BT_TEMP_DIR}/scripts/sdk/gn/base/bin/devshell/lib/emu-ifup-macos.sh"
      gn-test-check-mock-partial -device e1000,netdev=net0,mac=52:54:00:4d:27:96
    else
      gn-test-check-mock-partial -netdev type=tap,ifname=fakenetwork,id=net0,script="${BT_TEMP_DIR}/scripts/sdk/gn/base/bin/devshell/lib/emu-ifup-macos.sh"
      gn-test-check-mock-partial -device e1000,netdev=net0,mac=52:54:00:95:03:66
    fi
  else
    gn-test-check-mock-partial -netdev type=tap,ifname=qemu,id=net0,script=no
    gn-test-check-mock-partial -device e1000,netdev=net0,mac=52:54:00:63:5e:7a
  fi
  gn-test-check-mock-partial --unknown-arg1-to-qemu
  gn-test-check-mock-partial --unknown-arg2-to-qemu
}
# Test initialization. Note that we copy various tools/devshell files and need to replicate the
# behavior of generate.py by copying these files into scripts/sdk/gn/base/bin/devshell
# shellcheck disable=SC2034
BT_FILE_DEPS=(
  scripts/sdk/gn/base/bin/femu.sh
  scripts/sdk/gn/base/bin/devshell/lib/image_build_vars.sh
  scripts/sdk/gn/base/bin/fuchsia-common.sh
  scripts/sdk/gn/base/bin/fx-image-common.sh
  scripts/sdk/gn/bash_tests/gn-bash-test-lib.sh
  tools/devshell/emu
  tools/devshell/lib/fvm.sh
  tools/devshell/lib/emu-ifup-macos.sh
)
# shellcheck disable=SC2034
BT_MOCKED_TOOLS=(
  scripts/sdk/gn/base/images/emulator/aemu-"${AEMU_PLATFORM}"-"${AEMU_LABEL}"/emulator
  scripts/sdk/gn/base/bin/fpave.sh
  scripts/sdk/gn/base/bin/fserve.sh
  scripts/sdk/gn/base/tools/zbi
  scripts/sdk/gn/base/tools/fvm
  _isolated_path_for/ip
  # Create fake "stty sane" command so that fx emu test succeeds when < /dev/null is being used
  _isolated_path_for/stty
)

BT_SET_UP() {
  FUCHSIA_WORK_DIR="${BT_TEMP_DIR}/scripts/sdk/gn/base/images"

  # Create a small disk image to avoid downloading, and test if it is doubled in size as expected
  mkdir -p "${FUCHSIA_WORK_DIR}/image"
  dd if=/dev/zero of="${FUCHSIA_WORK_DIR}/image/storage-full.blk" bs=1024 count=1  > /dev/null 2>/dev/null
}

BT_INIT_TEMP_DIR() {

  # Generate the aemu.version file based on the simulated version string
  echo "${AEMU_VERSION}" > "${BT_TEMP_DIR}/scripts/sdk/gn/base/bin/aemu.version"


  # Create empty authorized_keys file to add to the system image, but the contents are not used.
  mkdir -p "${BT_TEMP_DIR}/scripts/sdk/gn/base/testdata"
  echo ssh-ed25519 00000000000000000000000000000000000000000000000000000000000000000000 \
    >"${BT_TEMP_DIR}/scripts/sdk/gn/base/testdata/authorized_keys"

  # Stage the files we copy from the fx emu implementation, replicating behavior of generate.py
  cp -r "${BT_TEMP_DIR}/tools/devshell" "${BT_TEMP_DIR}/scripts/sdk/gn/base/bin/"
}

BT_RUN_TESTS "$@"
