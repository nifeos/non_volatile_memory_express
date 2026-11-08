//! Fuzz the Controller Capabilities (`CAP`) decode with an arbitrary register value, as a hostile
//! or broken controller would report it, and the doorbell offsets and page pointers a driver
//! derives from it.
//!
//! The input's first 8 bytes are the register, the next 2 a queue identifier, and the next 16 a
//! buffer address and a transfer length. The target holds each field to its range, every doorbell
//! offset to the stride the register states, and the page pointer pair built against the
//! register's minimum page size to the rules of section 4.3.

#![no_main]

use libfuzzer_sys::fuzz_target;
use non_volatile_memory_express::{
    Capabilities, Doorbell, doorbell, physical_region_page_pair, registers,
};

fuzz_target!(|data: &[u8]| {
    let Some(bytes) = data.get(..26) else {
        return;
    };
    let register = u64::from_le_bytes(bytes[0..8].try_into().unwrap());
    let queue_id = u16::from_le_bytes(bytes[8..10].try_into().unwrap());
    let address = u64::from_le_bytes(bytes[10..18].try_into().unwrap());
    let length = u64::from_le_bytes(bytes[18..26].try_into().unwrap());
    let capabilities = Capabilities(register);

    let entries = capabilities.maximum_queue_entries();
    assert!((1..=1 << 16).contains(&entries));
    assert!(capabilities.timeout_in_500_millisecond_units() <= 0xff);
    let stride = capabilities.doorbell_stride();
    assert!(stride <= 0xf);
    let page = capabilities.minimum_page_size();
    assert!(page.is_power_of_two() && (1 << 12..=1 << 27).contains(&page));

    let spacing = 4u64 << stride;
    let submission = doorbell(queue_id, Doorbell::SubmissionTail, stride);
    let completion = doorbell(queue_id, Doorbell::CompletionHead, stride);
    assert_eq!(
        submission,
        registers::FIRST_DOORBELL_REGISTER + 2 * u64::from(queue_id) * spacing
    );
    assert_eq!(completion, submission + spacing);

    let Some(pair) = physical_region_page_pair(address, length, page) else {
        return;
    };
    let offset = address & (page - 1);
    assert!(length >= 1 && offset + length <= 2 * page);
    assert_eq!(pair.first_entry, address);
    if offset + length <= page {
        assert_eq!(pair.second_entry, 0);
    } else {
        assert_eq!(pair.second_entry & (page - 1), 0, "PRP2 carries no offset");
        assert_eq!(pair.second_entry, (address & !(page - 1)) + page);
    }
});
