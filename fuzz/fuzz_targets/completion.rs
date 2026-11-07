//! Fuzz the completion queue entry decode with arbitrary entries, as a hostile or broken controller
//! would write them into the ring.
//!
//! Every 16 bytes of input are one entry, read as four little-endian dwords. The target holds each
//! decoded field to the bits the specification gives it, and drives a completion ring with the
//! entries in order: an entry is consumed only when its phase tag matches the lap, and the head
//! stays inside the ring and flips the expected phase exactly at each wrap.
//!
//! Alongside it runs a submission ring of the same depth. Before each entry the driver submits as
//! many commands as the low byte of dword 1 asks (a reserved dword the controller should leave
//! zero, so it is free for the fuzzer to steer), then records the entry's submission queue head,
//! whatever the controller put there. `record_head` must accept exactly the heads that lie from the
//! recorded head forward to the tail, and refuse every other value without changing the ring.

#![no_main]

use libfuzzer_sys::fuzz_target;
use non_volatile_memory_express::{Completion, CompletionQueue, SubmissionQueue};

fuzz_target!(|data: &[u8]| {
    let Some((&depth, entries)) = data.split_first() else {
        return;
    };
    let slots = u16::from(depth).max(2);
    let mut queue = CompletionQueue::new(slots);
    let mut submission = SubmissionQueue::new(slots);
    let mut consumed = 0u32;
    for entry in entries.chunks_exact(16) {
        let dword = |i: usize| u32::from_le_bytes(entry[4 * i..4 * i + 4].try_into().unwrap());
        let dwords = [dword(0), dword(1), dword(2), dword(3)];
        let completion = Completion::from_dwords(dwords);

        assert_eq!(completion.result, dwords[0]);
        assert_eq!(
            u32::from(completion.submission_queue_head),
            dwords[2] & 0xffff
        );
        assert_eq!(u32::from(completion.submission_queue_id), dwords[2] >> 16);
        assert_eq!(u32::from(completion.command_id), dwords[3] & 0xffff);
        assert_eq!(completion.phase, dwords[3] & (1 << 16) != 0);
        assert_eq!(u32::from(completion.status), dwords[3] >> 17);
        assert!(completion.status < 1 << 15);
        assert_eq!(Completion::from_dwords(dwords), completion);

        for _ in 0..dwords[1] & 0xff {
            let tail = submission.tail();
            match submission.push() {
                Some(slot) => assert_eq!(slot, tail),
                None => assert_eq!(
                    (tail + 1) % slots,
                    submission.head(),
                    "refused a non-full ring"
                ),
            }
        }
        let ring = u32::from(slots);
        let ahead = |slot: u16| (u32::from(slot) + ring - u32::from(submission.head())) % ring;
        let reported = completion.submission_queue_head;
        let reachable = reported < slots && ahead(reported) <= ahead(submission.tail());
        let before = submission;
        match submission.record_head(reported) {
            Ok(()) => {
                assert!(
                    reachable,
                    "accepted a head the controller cannot have reached"
                );
                assert_eq!(submission.head(), reported);
                assert_eq!(submission.tail(), before.tail());
            }
            Err(error) => {
                assert!(!reachable, "refused a head the controller can have reached");
                assert_eq!(error.head, reported);
                assert_eq!(submission, before, "a refused head changed the ring");
            }
        }

        if !queue.is_owned(&completion) {
            continue;
        }
        let before = queue.head();
        let after = queue.pop();
        consumed += 1;
        assert!(after < slots);
        assert_eq!(after, queue.head());
        assert_eq!(after, (before + 1) % slots);
        // The expected phase flips exactly when the head wraps to slot 0.
        assert_eq!(queue.is_owned(&completion), after != 0);
    }
    assert_eq!(u32::from(queue.head()), consumed % u32::from(slots));
});
