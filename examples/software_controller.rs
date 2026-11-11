//! A controller in software, over two `Vec`s, driven through several laps of both rings.
//!
//! The controller consumes the submission ring up to the tail the driver rings and writes one
//! completion per command, with the phase of its lap. The driver mints each command identifier,
//! checks every completion's head and identifier, and pops until it meets a stale entry. Copy
//! `SoftwareController` as a test double; `cargo test` runs the loop as a test. Run it with
//! `cargo run --example software_controller` to see each submission queue head reported.

use non_volatile_memory_express::{
    Command, Completion, CompletionQueue, InFlight, SubmissionQueue, physical_region_page_pair,
};

const ENTRIES: u16 = 4;

/// The device side: the two rings in shared memory, and the controller's own head, tail and phase.
struct SoftwareController {
    submission_ring: Vec<[u32; 16]>,
    completion_ring: Vec<[u32; 4]>,
    submission_head: u16,
    completion_tail: u16,
    phase: bool,
}

impl SoftwareController {
    fn new() -> Self {
        SoftwareController {
            submission_ring: vec![[0; 16]; usize::from(ENTRIES)],
            completion_ring: vec![[0; 4]; usize::from(ENTRIES)],
            submission_head: 0,
            completion_tail: 0,
            phase: true,
        }
    }

    /// The driver wrote `tail` to the submission tail doorbell: consume every command up to it.
    fn ring_submission_tail(&mut self, tail: u16) {
        while self.submission_head != tail {
            let command = self.submission_ring[usize::from(self.submission_head)];
            self.submission_head = (self.submission_head + 1) % ENTRIES;
            let head = u32::from(self.submission_head);
            self.complete([
                0,
                0,
                head | 1 << 16,
                command[0] >> 16 | u32::from(self.phase) << 16,
            ]);
        }
    }

    /// Write one completion at the tail, flipping the phase when the tail wraps.
    fn complete(&mut self, dwords: [u32; 4]) {
        self.completion_ring[usize::from(self.completion_tail)] = dwords;
        self.completion_tail = (self.completion_tail + 1) % ENTRIES;
        if self.completion_tail == 0 {
            self.phase = !self.phase;
        }
    }
}

/// Run `batches` rounds of one to three reads each and return every head the driver recorded, and
/// the error from the one impossible head the controller reports at the end.
fn run(batches: usize) -> (Vec<u16>, String) {
    let mut controller = SoftwareController::new();
    let mut submission_queue = SubmissionQueue::new(ENTRIES);
    let mut completion_queue = CompletionQueue::new(ENTRIES);
    let mut in_flight = InFlight::<1>::new();
    let mut heads = Vec::new();
    for batch in 0..batches {
        for block in 0..batch % 3 + 1 {
            let command_id = in_flight
                .mint()
                .expect("at most three commands are in flight");
            let pages = physical_region_page_pair(0x10_0000, 4096, 4096).expect("one page");
            let read = Command::read(command_id, 1, block as u64 * 8, 8, pages);
            let slot = submission_queue
                .push()
                .expect("a ring of four holds three commands");
            controller.submission_ring[usize::from(slot)] = read.0;
        }
        controller.ring_submission_tail(submission_queue.tail());
        loop {
            let dwords = controller.completion_ring[usize::from(completion_queue.head())];
            if Completion::is_phase_tag_set(dwords[3]) != completion_queue.expects_phase_tag_set() {
                break; // a stale entry, from the last lap or the initial zeros: nothing more yet
            }
            let completion = Completion::from_dwords(dwords);
            assert!(completion.is_success() && completion.submission_queue_id == 1);
            in_flight
                .release(completion.command_id)
                .expect("the controller completes only commands it was given");
            submission_queue
                .record_head(completion.submission_queue_head)
                .expect("the controller reports only heads it has reached");
            heads.push(completion.submission_queue_head);
            completion_queue.pop();
        }
        assert!(in_flight.is_empty() && submission_queue.head() == submission_queue.tail());
    }
    // A broken controller reports a head one past the tail, a command it was never given.
    let impossible = (submission_queue.tail() + 1) % ENTRIES;
    let before = submission_queue;
    let error = submission_queue.record_head(impossible).unwrap_err();
    assert_eq!(submission_queue, before, "a refused head changes nothing");
    (heads, error.to_string())
}

fn main() {
    let (heads, error) = run(6);
    println!("submission queue heads reported: {heads:?}");
    println!("refused: {error}");
}

#[test]
fn both_rings_lap_and_one_impossible_head_is_refused() {
    // Six batches of 1, 2, 3, 1, 2 and 3 commands: twelve completions, three laps of each ring.
    let (heads, error) = run(6);
    assert_eq!(heads, [1, 2, 3, 0, 1, 2, 3, 0, 1, 2, 3, 0]);
    assert_eq!(
        error,
        "the controller reported submission queue head 1, which it cannot have reached"
    );
}
