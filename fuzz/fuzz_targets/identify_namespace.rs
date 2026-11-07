//! Fuzz the Identify namespace parse with an arbitrary structure, as a hostile or broken controller
//! would return it.
//!
//! The input's first 24 bytes choose what a driver asks next (a block number, a buffer address, a
//! direction), and the rest is the structure. The target holds the two parse functions to each
//! other and to what they promise: `parse_identify_namespace` accepts only a shift of 9 to 12 and
//! then agrees with `logical_block_format_size_shift`, a 4096-byte unit always covers 1 to 8
//! logical blocks, and the namespace built from the answer builds a transfer command only for a
//! block it holds.

#![no_main]

use libfuzzer_sys::fuzz_target;
use non_volatile_memory_express::{
    Namespace, logical_block_format_size_shift, parse_identify_namespace,
};

/// The transfer unit this crate serves: one 4096-byte block.
const UNIT: u64 = 4096;

fuzz_target!(|data: &[u8]| {
    let Some((request, structure)) = data.split_at_checked(24) else {
        return;
    };
    let word = |at: usize| u64::from_le_bytes(request[at..at + 8].try_into().unwrap());
    let (block, address, flags) = (word(0), word(8), word(16));

    let reported = logical_block_format_size_shift(structure);
    assert_eq!(reported.is_some(), structure.len() >= 384);
    let Some(namespace) = parse_identify_namespace(structure) else {
        return;
    };
    assert_eq!(reported, Some(namespace.logical_block_size_shift as u8));
    assert!((9..=12).contains(&namespace.logical_block_size_shift));
    assert_eq!(
        namespace.blocks,
        u64::from_le_bytes(structure[0..8].try_into().unwrap())
    );

    let per_transfer = namespace
        .blocks_per_transfer(UNIT)
        .expect("every accepted shift divides a 4096-byte unit");
    assert!((1..=8).contains(&per_transfer));
    assert_eq!(namespace.blocks_per_transfer(0), None);

    let device = Namespace::new(namespace.capacity_bytes(), per_transfer);
    let held = device.holds_whole_block(block, UNIT);
    let command = device.transfer_command(
        flags as u16,
        (flags >> 16) as u32,
        block,
        UNIT,
        address,
        UNIT,
        flags & (1 << 63) != 0,
    );
    if command.is_some() {
        assert!(held, "a command for a block the namespace does not hold");
    }
    if held && address & (UNIT - 1) == 0 {
        assert!(command.is_some(), "a held, aligned block built no command");
    }
});
