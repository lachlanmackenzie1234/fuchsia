// Copyright 2026 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

use sapphire_emboss::example::ExampleHeader;
use sapphire_emboss::example2::Example2Packet;

#[test]
fn test_example_header() {
    let buffer = [0x42u8, 0x10u8];
    let view = ExampleHeader::new(&buffer[..]);
    assert_eq!(view.opcode().try_read().unwrap(), 0x42);
    assert_eq!(view.length().try_read().unwrap(), 0x10);
}

#[test]
fn test_example2_packet_with_imported_example() {
    let buffer = [0x42u8, 0x10u8, 0x05u8, 0x00u8];
    let view = Example2Packet::new(&buffer[..]);
    assert_eq!(view.header().opcode().try_read().unwrap(), 0x42);
    assert_eq!(view.header().length().try_read().unwrap(), 0x10);
    assert_eq!(view.payload_id().try_read().unwrap(), 5);
}
