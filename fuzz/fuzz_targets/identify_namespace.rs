//! Fuzz the Identify namespace parse with an arbitrary structure, as a hostile or broken controller
//! would return it.
//!
//! The input's first 32 bytes choose what a driver asks next (a block number, a buffer address, a
//! direction, a transfer unit of any size), and the rest is the structure. The target holds the two
//! parse functions to each other and to what they promise: `parse_identify_namespace` accepts only
//! a shift of 9 to 12 and then agrees with `logical_block_format_size_shift`, a 4096-byte unit
//! always covers 1 to 8 logical blocks, and the namespace built from the answer builds a transfer
//! command only for a block it holds. Any other unit, including one of 65536 logical blocks or
//! more, either covers a nonzero count of whole blocks that a transfer command can carry, or is
//! refused.

#![no_main]

use libfuzzer_sys::fuzz_target;
use non_volatile_memory_express::{
    Namespace, logical_block_format_size_shift, parse_identify_namespace,
};

/// The transfer unit this crate serves: one 4096-byte block.
const UNIT: u64 = 4096;

fuzz_target!(|data: &[u8]| {
    let Some((request, structure)) = data.split_at_checked(32) else {
        return;
    };
    let word = |at: usize| u64::from_le_bytes(request[at..at + 8].try_into().unwrap());
    let (block, address, flags, any_unit) = (word(0), word(8), word(16), word(24));

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

    // A unit of any size, and the two either side of the largest count a command carries. The
    // fuzzer would rarely land on those by chance, so they are asked every time.
    let shift = namespace.logical_block_size_shift;
    let largest = u64::from(u16::MAX) << shift;
    for unit in [any_unit, largest, largest + (1 << shift)] {
        let whole = unit != 0 && unit & ((1 << shift) - 1) == 0;
        let fits = unit >> shift <= u64::from(u16::MAX);
        match namespace.blocks_per_transfer(unit) {
            Some(count) => {
                assert!(whole && fits, "a count for a unit it cannot serve");
                assert!(count >= 1);
                assert_eq!(u64::from(count) << shift, unit, "a truncated count");
                // The count reaches the command builders, which refuse a zero, and must not
                // panic for any block or address.
                let device = Namespace::new(namespace.capacity_bytes(), count);
                let _ = device.transfer_command(1, 1, block, unit, address, UNIT, flags & 1 != 0);
            }
            None => assert!(!(whole && fits), "refused a unit it can serve"),
        }
    }
});
