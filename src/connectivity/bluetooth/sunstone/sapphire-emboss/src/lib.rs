// Copyright 2026 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

pub use pw_bluetooth_hci_commands_emb as hci_commands;
pub use pw_bluetooth_hci_common_emb as hci_common;
pub use pw_bluetooth_hci_h4_emb as hci_h4;
pub use pw_bluetooth_l2cap_frames_emb as l2cap_frames;

#[cfg(test)]
mod tests {
    use super::*;
    use hci_commands::ResetCommand;
    use hci_common::OpCode;

    #[test]
    fn test_reset_command() {
        let buffer = [0x03u8, 0x0cu8, 0x00u8];
        let view = ResetCommand::new(&buffer[..]);
        assert_eq!(view.header().opcode().try_read().unwrap(), OpCode::RESET);
        assert_eq!(view.header().parameter_total_size().try_read().unwrap(), 0);
    }
}
