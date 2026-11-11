//! Machine-checked proofs (`cargo kani`).
//!
//! The ring arithmetic runs forever against a device that writes into the same memory, so the
//! properties proved are the ones a lapped or hostile ring could otherwise erode: indices stay
//! inside the ring for every reachable state, the phase discipline is exactly one flip per lap,
//! the doorbell and PRP arithmetic is total (no register value the controller reports can make an
//! offset computation panic or two doorbells collide), no transfer command addresses a block
//! outside its namespace, and the two decoders of words the controller writes (the capabilities
//! register and the completion status) read exactly the bits the specification gives them.

use crate::*;

/// Falsification: replayable `falsifications/verification.completion_pop_stays_in_bounds_and_flips_only_at_the_wrap.patch`
/// **The completion head stays in bounds and flips the phase exactly at the wrap**, for every
/// reachable state, not just the ones a test happened to walk.
#[kani::proof]
fn completion_pop_stays_in_bounds_and_flips_only_at_the_wrap() {
    let entries: u16 = kani::any();
    let head: u16 = kani::any();
    let phase: bool = kani::any();
    kani::assume(entries >= 2 && head < entries);
    let mut queue = CompletionQueue {
        entries,
        head,
        phase,
    };
    let popped = queue.pop();
    assert!(queue.head < entries);
    assert_eq!(popped, queue.head);
    // The head advanced by exactly one, modulo the ring. Without this the two assertions above
    // hold for a pop that wraps one slot early (or late by a lap), which a device writing
    // completions into every slot would then never be read from.
    assert_eq!(
        u32::from(queue.head),
        (u32::from(head) + 1) % u32::from(entries)
    );
    // The phase flipped if and only if the head wrapped to zero.
    assert_eq!(queue.phase != phase, queue.head == 0);
}

/// Falsification: replayable `falsifications/verification.submission_push_stays_in_bounds.patch`
/// **A submission push lands in bounds from any state, and refuses exactly when the ring is
/// full.** A full ring answers `None` and changes nothing; any other ring hands back a slot inside
/// it and leaves a legal tail, which is the doorbell value the caller writes.
#[kani::proof]
fn submission_push_stays_in_bounds() {
    let entries: u16 = kani::any();
    let tail: u16 = kani::any();
    let head: u16 = kani::any();
    kani::assume(entries >= 2 && tail < entries && head < entries);
    let full = (tail + 1) % entries == head;
    let mut queue = SubmissionQueue {
        entries,
        tail,
        head,
    };
    let before = queue;
    match queue.push() {
        Some(slot) => {
            assert!(!full);
            assert!(slot < entries);
            assert!(queue.tail() < entries);
        }
        None => {
            assert!(full);
            assert_eq!(queue, before);
        }
    }
}

/// Falsification: replayable `falsifications/verification.distinct_doorbells_never_collide.patch`
/// **No two doorbells collide**, for any queue ids and any stride the controller can report:
/// distinct (queue, role) pairs always compute distinct offsets, so a tail write can never
/// land on a head register. This is the arithmetic the caller's volatile writes trust.
#[kani::proof]
fn distinct_doorbells_never_collide() {
    let first_queue_id: u16 = kani::any();
    let second_queue_id: u16 = kani::any();
    let first_role = if kani::any() {
        Doorbell::SubmissionTail
    } else {
        Doorbell::CompletionHead
    };
    let second_role = if kani::any() {
        Doorbell::SubmissionTail
    } else {
        Doorbell::CompletionHead
    };
    let doorbell_stride: u32 = kani::any();
    if (first_queue_id, first_role) != (second_queue_id, second_role) {
        assert!(
            doorbell(first_queue_id, first_role, doorbell_stride)
                != doorbell(second_queue_id, second_role, doorbell_stride)
        );
    }
}

/// Falsification: replayable `falsifications/verification.physical_region_page_pair_is_total_and_page_disciplined.patch`
/// **PRP construction is total and never spans more pages than it names.** For any base,
/// length and power-of-two page size, `physical_region_page_pair` either refuses or returns a pair
/// whose two pointers cover the whole transfer: the second entry is zero exactly when one page
/// suffices, and is page-aligned whenever it is used (the specification's requirement on every
/// pointer after the first).
#[kani::proof]
fn physical_region_page_pair_is_total_and_page_disciplined() {
    let base: u64 = kani::any();
    let length: u64 = kani::any();
    let shift: u32 = kani::any();
    kani::assume((9..=16).contains(&shift));
    let page = 1u64 << shift;
    if let Some(pair) = physical_region_page_pair(base, length, page) {
        assert_eq!(pair.first_entry, base);
        let offset = base & (page - 1);
        if offset + length <= page {
            assert_eq!(pair.second_entry, 0);
        } else {
            assert_eq!(pair.second_entry & (page - 1), 0);
            assert_eq!(pair.second_entry, (base & !(page - 1)) + page);
        }
    }
}

/// Falsification: replayable `falsifications/verification.a_transfer_command_is_only_built_for_a_block_the_namespace_has.patch`
/// **No transfer command is built for a block outside its namespace, and none overflows
/// computing one.** This is the bounds check a confined driver leans on: an input-output memory management unit (IOMMU) bounds where
/// the controller may *write*, and nothing but this bounds *which LBA* is asked for, so a
/// caller-supplied `u64` must not be able to wrap into a valid-looking address.
#[kani::proof]
fn a_transfer_command_is_only_built_for_a_block_the_namespace_has() {
    let blocks_per_transfer: u16 = kani::any();
    kani::assume((1..=8).contains(&blocks_per_transfer));
    let Some(namespace) = Namespace::new(kani::any(), 4096, blocks_per_transfer) else {
        // A unit of 4096 bytes is no whole number of 3-, 5-, 6- or 7-block logical blocks.
        assert!(4096 % blocks_per_transfer != 0);
        return;
    };
    let block: u64 = kani::any();
    let data_physical_address: u64 = kani::any();
    kani::assume(data_physical_address % 4096 == 0 && data_physical_address < u64::MAX - 8192);
    let direction = if kani::any() {
        Direction::Read
    } else {
        Direction::Write
    };
    if namespace
        .transfer_command(1, 1, block, data_physical_address, 4096, direction)
        .is_some()
    {
        assert!(namespace.holds_whole_block(block));
        // The transfer's last byte is inside the namespace, which is the property
        // `holds_whole_block` exists to state and the one a wrap would quietly break.
        assert!((block + 1) * 4096 <= namespace.capacity_bytes());
    }
}

/// Falsification: replayable `falsifications/verification.a_transfer_command_names_only_logical_blocks_the_namespace_has.patch`
/// **Every logical block a transfer command names lies inside the namespace.** The byte bound
/// above is stated in transfer units; the command itself carries a starting logical block and a
/// count, and those are what the controller reads. For a unit of 4096 bytes and any block count
/// per unit a 512-byte to 4096-byte format gives, the last logical block an accepted command
/// names is below the namespace's logical block count, so the unit and the count cannot disagree.
#[kani::proof]
fn a_transfer_command_names_only_logical_blocks_the_namespace_has() {
    let shift: u32 = kani::any();
    kani::assume(shift <= 3);
    let blocks_per_transfer = 1u16 << shift;
    let namespace = Namespace::new(kani::any(), 4096, blocks_per_transfer).unwrap();
    let block: u64 = kani::any();
    let direction = if kani::any() {
        Direction::Read
    } else {
        Direction::Write
    };
    if let Some(command) = namespace.transfer_command(1, 1, block, 0x1000, 4096, direction) {
        let starting_logical_block = u64::from(command.0[10]) | u64::from(command.0[11]) << 32;
        let blocks = u64::from(command.0[12] & 0xffff) + 1;
        assert_eq!(blocks, u64::from(blocks_per_transfer));
        let logical_block_bytes = 4096 / u64::from(blocks_per_transfer);
        let namespace_blocks = namespace.capacity_bytes() / logical_block_bytes;
        assert!(starting_logical_block + blocks <= namespace_blocks);
    }
}

/// Falsification: replayable `falsifications/verification.identify_parse_is_total_and_bounds_the_shift.patch`
/// **Identify parsing is total for any device response.** The 4096 bytes come from the
/// controller's DMA; no contents may panic the parse, and an accepted answer always carries a
/// shift the block driver's arithmetic (`blocks_per_transfer(4096)`) can serve.
#[kani::proof]
fn identify_parse_is_total_and_bounds_the_shift() {
    let data: [u8; 384] = kani::any();
    if let Some(id) = parse_identify_namespace(&data) {
        assert!((9..=12).contains(&id.logical_block_size_shift));
        let per = id.blocks_per_transfer(4096).unwrap();
        assert!((1..=8).contains(&per));
    }
}

/// Falsification: replayable `falsifications/verification.capabilities_decode_is_total_and_reads_the_specified_bits.patch`
/// **The capabilities decoder is total and reads exactly the specification's bit positions**, for
/// any 64-bit register value a controller may report. Each field lies in the range its width
/// allows (the queue depth from 1 to 65536, the doorbell stride below 16, the page size a power of
/// two from 4 KiB to 128 MiB), and each equals the bits at its place in the register, so a shift
/// or mask moved by one bit is caught.
#[kani::proof]
fn capabilities_decode_is_total_and_reads_the_specified_bits() {
    let register: u64 = kani::any();
    let capabilities = Capabilities(register);

    let depth = capabilities.maximum_queue_entries();
    assert!((1..=65_536).contains(&depth));
    assert_eq!(u64::from(depth), (register & 0xffff) + 1);

    assert!(capabilities.doorbell_stride() <= 15);
    assert_eq!(
        u64::from(capabilities.doorbell_stride()),
        (register >> 32) & 0xf
    );

    assert!(capabilities.timeout_in_500_millisecond_units() <= 255);
    assert_eq!(
        u64::from(capabilities.timeout_in_500_millisecond_units()),
        (register >> 24) & 0xff
    );

    let page = capabilities.minimum_page_size_bytes();
    assert!(page.is_power_of_two());
    assert!((4096..=1u64 << 27).contains(&page));
    assert_eq!(page.trailing_zeros() as u64, 12 + ((register >> 48) & 0xf));

    let page = capabilities.maximum_page_size_bytes();
    assert!(page.is_power_of_two());
    assert!((4096..=1u64 << 27).contains(&page));
    assert_eq!(page.trailing_zeros() as u64, 12 + ((register >> 52) & 0xf));
}

/// Falsification: replayable `falsifications/verification.controller_configuration_round_trips_the_page_size.patch`
/// **The configuration word carries exactly the page size it was asked for.** For any byte count,
/// `enabled_controller_configuration` answers exactly when the count is a power of two from 4 KiB
/// to 128 MiB, and the answer's Memory Page Size field (bits 7:10) decodes back to that count,
/// with the enable bit and the two entry sizes set and no other bit. Every page size a
/// capabilities register names as its minimum or its maximum is one it accepts.
#[kani::proof]
fn controller_configuration_round_trips_the_page_size() {
    let page_size_bytes: u64 = kani::any();
    let nameable =
        page_size_bytes.is_power_of_two() && (4096..=1u64 << 27).contains(&page_size_bytes);
    match enabled_controller_configuration(page_size_bytes) {
        Some(configuration) => {
            assert!(nameable);
            assert_eq!(1u64 << (12 + ((configuration >> 7) & 0xf)), page_size_bytes);
            assert_eq!(configuration & !(0xf << 7), 0x0046_0001);
        }
        None => assert!(!nameable),
    }

    let capabilities = Capabilities(kani::any());
    assert!(enabled_controller_configuration(capabilities.minimum_page_size_bytes()).is_some());
    assert!(enabled_controller_configuration(capabilities.maximum_page_size_bytes()).is_some());
}

/// Falsification: replayable `falsifications/verification.admin_queue_attributes_round_trip_the_two_sizes.patch`
/// **The admin queue sizes reach the controller as the counts the driver built its rings with.**
/// For any two counts from 2 to 4096, the submission size decodes from bits 0:11 and the
/// completion size from bits 16:27, each plus one, back to the count asked for, and no reserved
/// bit is set. A ring the driver builds with the same count therefore wraps where the controller's
/// does.
#[kani::proof]
fn admin_queue_attributes_round_trip_the_two_sizes() {
    let submission_entries: u16 = kani::any();
    let completion_entries: u16 = kani::any();
    kani::assume((2..=4096).contains(&submission_entries));
    kani::assume((2..=4096).contains(&completion_entries));
    let attributes = admin_queue_attributes(submission_entries, completion_entries);
    assert_eq!((attributes & 0xfff) + 1, submission_entries as u32);
    assert_eq!(((attributes >> 16) & 0xfff) + 1, completion_entries as u32);
    assert_eq!(attributes & 0xf000_f000, 0);
}

/// Falsification: replayable `falsifications/verification.a_shutdown_notification_changes_only_its_field.patch`
/// **A shutdown notification is normal and touches nothing else.** For any configuration word the
/// driver last wrote, `shutdown_controller_configuration` leaves Shutdown Notification (bits
/// 14:15) at 01b, the normal notification, whatever the field held before (an abrupt 10b or the
/// reserved 11b included), and keeps every other bit, so the enable bit, the page size and the
/// entry sizes stay as they were. The two Shutdown Status bits lie in the field the specification
/// gives them (bits 2:3) and share no bit with the ready or fatal bit.
#[kani::proof]
fn a_shutdown_notification_changes_only_its_field() {
    let configuration: u32 = kani::any();
    let shutdown = shutdown_controller_configuration(configuration);
    assert_eq!((shutdown >> 14) & 0b11, 0b01);
    assert_eq!(shutdown & !(0b11 << 14), configuration & !(0b11 << 14));

    let status = CONTROLLER_STATUS_SHUTDOWN_OCCURRING | CONTROLLER_STATUS_SHUTDOWN_COMPLETE;
    assert_eq!(status, 0b11 << 2);
    assert_eq!(
        status & (CONTROLLER_STATUS_READY | CONTROLLER_STATUS_FATAL),
        0
    );
}

/// Falsification: replayable `falsifications/verification.completion_decode_is_total_and_rebuilds_dword_three.patch`
/// **The completion status decoder is total and reads exactly the specification's bit
/// positions**, for any four dwords a controller may write. The status field is the 15 bits above
/// the phase tag, the phase tag is bit 16, and together with the command identifier they rebuild
/// dword 3 exactly, so no bit of it is dropped, doubled or read from the wrong place. A zero
/// status is success, and only an all-zero field is.
#[kani::proof]
fn completion_decode_is_total_and_rebuilds_dword_three() {
    let dwords: [u32; 4] = kani::any();
    let completion = Completion::from_dwords(dwords);

    assert!(completion.status < 1 << 15);
    let rebuilt = u32::from(completion.command_id)
        | u32::from(completion.phase) << 16
        | u32::from(completion.status) << 17;
    assert_eq!(rebuilt, dwords[3]);
    assert_eq!(completion.status == 0, dwords[3] >> 17 == 0);

    assert_eq!(completion.result, dwords[0]);
    assert_eq!(
        u32::from(completion.submission_queue_head)
            | u32::from(completion.submission_queue_id) << 16,
        dwords[2]
    );
}

/// Falsification: replayable `falsifications/verification.completion_status_parts_rebuild_the_status_field.patch`
/// **Each part of a completion's status reads its own bits and no others**, for any dword 3 a
/// controller may write. The status code (8 bits), the code type (3), the retry delay (2), More
/// and Do Not Retry lie in the ranges their widths allow and, shifted back to their places,
/// rebuild the 15-bit field exactly, so no part reads a neighbor's bit. Success is exactly an
/// all-zero field, and the phase tag read from dword 3 alone, before the rest of the entry, is the
/// one the full decode gives.
#[kani::proof]
fn completion_status_parts_rebuild_the_status_field() {
    let dword_three: u32 = kani::any();
    let completion = Completion::from_dwords([0, 0, 0, dword_three]);

    assert!(completion.status_code_type() < 1 << 3);
    assert!(completion.command_retry_delay() < 1 << 2);
    let rebuilt = u16::from(completion.status_code())
        | u16::from(completion.status_code_type()) << 8
        | u16::from(completion.command_retry_delay()) << 11
        | u16::from(completion.more()) << 13
        | u16::from(completion.do_not_retry()) << 14;
    assert_eq!(rebuilt, completion.status);
    assert_eq!(completion.is_success(), dword_three >> 17 == 0);
    assert_eq!(Completion::phase_of(dword_three), completion.phase);
}

/// Falsification: replayable `falsifications/verification.record_head_accepts_only_a_head_the_controller_can_have_reached.patch`
/// **No head a controller reports can panic the submission ring or corrupt it.** For any ring
/// state and any reported head, `record_head` accepts exactly the heads that lie from the recorded
/// head forward, round the ring, to the tail, and records them; every other value is refused with
/// the ring unchanged, so a later push can never hand out a slot the controller has not read.
#[kani::proof]
fn record_head_accepts_only_a_head_the_controller_can_have_reached() {
    let entries: u16 = kani::any();
    let tail: u16 = kani::any();
    let head: u16 = kani::any();
    kani::assume(entries >= 2 && tail < entries && head < entries);
    let mut queue = SubmissionQueue {
        entries,
        tail,
        head,
    };
    let before = queue;
    let reported: u16 = kani::any();
    let ring = u32::from(entries);
    let ahead = |slot: u16| (u32::from(slot) + ring - u32::from(head)) % ring;
    let reachable = reported < entries && ahead(reported) <= ahead(tail);
    match queue.record_head(reported) {
        Ok(()) => {
            assert!(reachable);
            assert_eq!(queue.head(), reported);
            assert_eq!(queue.tail(), tail);
        }
        Err(error) => {
            assert!(!reachable);
            assert_eq!(error.head, reported);
            assert_eq!(queue, before);
        }
    }
}

/// Falsification: replayable `falsifications/verification.blocks_per_transfer_never_truncates_the_count.patch`
/// **The blocks-per-transfer count is exact or refused, never truncated.** For any unit and any
/// shift a parse accepts, an answer is at least 1 and covers exactly `unit` bytes, so no unit can
/// wrap the 16-bit count to 0 (which the read and write builders refuse) or to a smaller transfer.
#[kani::proof]
fn blocks_per_transfer_never_truncates_the_count() {
    let unit: u64 = kani::any();
    let shift: u32 = kani::any();
    kani::assume((9..=12).contains(&shift));
    let namespace = IdentifyNamespace {
        blocks: 0,
        logical_block_size_shift: shift,
    };
    match namespace.blocks_per_transfer(unit) {
        Some(blocks) => {
            assert!(blocks >= 1);
            assert_eq!(u64::from(blocks) << shift, unit);
        }
        None => {
            assert!(unit == 0 || unit % (1 << shift) != 0 || unit >> shift > u64::from(u16::MAX));
        }
    }
}
