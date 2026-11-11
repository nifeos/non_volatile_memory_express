# `non_volatile_memory_express`

[![check](https://github.com/nifeos/non_volatile_memory_express/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/nifeos/non_volatile_memory_express/actions/workflows/ci.yml?query=branch%3Amain)
[![proofs](https://github.com/nifeos/non_volatile_memory_express/actions/workflows/proofs.yml/badge.svg?branch=main)](https://github.com/nifeos/non_volatile_memory_express/actions/workflows/proofs.yml?query=branch%3Amain)
[![license](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](https://github.com/nifeos/non_volatile_memory_express#license)
[![minimum Rust version](https://img.shields.io/badge/rust-1.85%2B-blue.svg)](https://github.com/nifeos/non_volatile_memory_express/blob/main/Cargo.toml)

The queue mechanics of an `NVMe` (Non-Volatile Memory Express) driver, as pure logic. It covers
register field decode, command building, submission and completion ring arithmetic with the phase
tag, doorbell addressing, Physical Region Page (PRP) construction and Identify namespace and
controller parsing.
It follows revision 1.4 of the `NVMe` base specification. Every section number in this crate's
documentation is that revision's. The crate is `no_std`, needs no allocator and performs no I/O.
You supply the volatile memory-mapped input/output (MMIO) accessors, the direct memory access (DMA)
memory and the timing (reset, enable, timeouts). The crate computes every value you write and
checks every value the controller hands back.

A PRP entry is a physical address of one page of a transfer's data. A command carries two of them,
PRP1 and PRP2. A longer transfer points PRP2 at a list of more entries, which this crate does not
build. A logical block address (LBA) numbers the fixed-size blocks of a namespace, counting from 0.
A block is the unit of storage a controller exposes.

## EXAMPLES

```rust
use non_volatile_memory_express::{
    Command, Completion, CompletionQueue, Doorbell, SubmissionQueue, admin_queue_attributes,
    doorbell, physical_region_page_pair,
};

// Bring-up writes the admin ring sizes to the Admin Queue Attributes register before it enables
// the controller; the driver's own admin rings use the same counts.
assert_eq!(admin_queue_attributes(32, 32), 0x001f_001f);

let mut submission_queue = SubmissionQueue::new(64);
let mut completion_queue = CompletionQueue::new(64);

// Read eight 512-byte blocks from LBA 0 of namespace 1 into a buffer that straddles a page.
let pages = physical_region_page_pair(0x4000_0800, 4096, 4096).expect("two pages suffice");
let read = Command::read(1, 1, 0, 8, pages);
// Write `read.0` into submission slot `slot`, issue a write barrier, then ring the tail:
let slot = submission_queue.push().expect("the ring has room");
assert_eq!(doorbell(1, Doorbell::SubmissionTail, 0), 0x1008); // write the tail here
assert_eq!((slot, submission_queue.tail()), (0, 1));
assert_eq!(read.0[1], 1);

// Poll dword 3 of completion slot `completion_queue.head()` until its phase bit (bit 16) carries
// the expected phase, issue a read barrier, then read dwords 0 to 2. Here the controller reports
// submission queue head 1 (it consumed the command in slot 0) for submission queue 1, and echoes
// command identifier 1 with status 0.
let dword_three = 1 | (1 << 16);
assert_eq!(Completion::phase_of(dword_three), completion_queue.expected_phase());
let done = Completion::from_dwords([0, 0, 1 | (1 << 16), dword_three]);
assert!(completion_queue.is_owned(&done)); // the phase tag only
// Ownership says the entry is fresh, not that it answers a command this driver issued: match the
// identifiers against the commands in flight.
assert!(done.submission_queue_id == 1 && done.command_id == 1 && done.is_success());
// The head comes from the controller, so it is checked: an impossible one is an error to handle
// (here, by giving up), never a panic.
submission_queue
    .record_head(done.submission_queue_head)
    .expect("the controller consumed the command it was given");
// Write the new head to doorbell(1, Doorbell::CompletionHead, stride).
assert_eq!(completion_queue.pop(), 1);
```

## What is proven

To build and test, install Rust 1.85 or newer (for example with `rustup`), then run `cargo build`
and `cargo test` from the crate's directory. The test run also compiles the example above as a
documentation test.

To run the proofs, install Kani as its
[installation guide](https://model-checking.github.io/kani/install-guide.html) says:
`cargo install --locked kani-verifier --version 0.68.0`, then `cargo kani setup` once (it
downloads a pinned nightly toolchain). Run `cargo kani` from the crate's directory. The harnesses
pass on Kani 0.68.0; all seventeen take about 30 seconds in total on a laptop-class machine, after
the setup. To run one, add `--harness <name>`. Seventeen harnesses live in the `verification`
module:

- `submission_push_stays_in_bounds`: a push onto any ring returns a slot inside the ring and leaves
  a legal tail, or returns `None` and changes nothing, and it returns `None` exactly when the ring
  is full.
- `completion_pop_stays_in_bounds_and_flips_only_at_the_wrap`: a completion pop keeps the head
  inside the ring, and the expected phase flips exactly once per lap.
- `distinct_doorbells_never_collide`: two different (queue, role) pairs never share a doorbell
  offset, for every stride a controller may report.
- `physical_region_page_pair_is_total_and_page_disciplined`: for any base, length and page size,
  the pair either refuses or covers the transfer; the second entry is zero when one page suffices,
  and otherwise the page boundary right after the base.
- `a_transfer_command_is_only_built_for_a_block_the_namespace_has`: `Namespace::transfer_command`
  builds a command only for a block whose last byte lies inside the namespace, and never overflows
  computing its LBA, for any block number, buffer address and namespace size.
- `a_transfer_command_names_only_logical_blocks_the_namespace_has`: the starting logical block
  and the count an accepted transfer command carries end inside the namespace, for a 4096-byte
  unit over 512-byte to 4096-byte logical blocks, so the unit and the count cannot disagree.
- `identify_parse_is_total_and_bounds_the_shift`: no controller response panics the Identify
  namespace parse, and every accepted LBA shift lies between 9 and 12, so a 4 KiB transfer covers
  1 to 8 logical blocks.
- `identify_controller_parse_is_total_and_reads_its_bytes`: no controller response panics the
  Identify Controller parse. Every answer of 520 bytes or more is accepted, with the transfer limit
  read from byte 77 and the namespace count from bytes 516 to 519. The limit in bytes is the
  minimum page shifted by the exponent, or `u64::MAX` when there is none or it does not fit.
- `capabilities_decode_is_total_and_reads_the_specified_bits`: for any value of the capabilities
  register, every `Capabilities` field lies in the range its width allows and equals the bits at
  its place in the register.
- `controller_configuration_round_trips_the_page_size`: `enabled_controller_configuration` accepts
  exactly the powers of two from 4 KiB to 128 MiB, encodes each so that its Memory Page Size field
  decodes back to the same size, and accepts every minimum and maximum a capabilities register
  can name.
- `admin_queue_attributes_round_trip_the_two_sizes`: for any two counts from 2 to 4096,
  `admin_queue_attributes` packs each 0-based into its own 12-bit field, so both decode back to
  the counts asked for, and it sets no reserved bit.
- `a_shutdown_notification_changes_only_its_field`: for any configuration word,
  `shutdown_controller_configuration` leaves the Shutdown Notification field at the normal value
  01b, whatever it held, and keeps every other bit; the two Shutdown Status bits sit at bits 2:3.
- `completion_decode_is_total_and_rebuilds_dword_three`: for any four dwords, the command
  identifier, the phase tag and the 15-bit status field rebuild dword 3 exactly, so the decode
  loses no bit, doubles none and reads none from the wrong place.
- `completion_status_parts_rebuild_the_status_field`: for any dword 3, the status code, the code
  type, the retry delay, More and Do Not Retry each lie in the range their width allows. Together
  they rebuild the 15-bit status field. Success is exactly an all-zero field, and the phase read
  from dword 3 alone equals the decoded phase tag.
- `no_command_identifier_releases_twice`: for any set of identifiers in flight, `InFlight::release`
  frees an identifier exactly when it is in flight, so a second completion for it is refused.
  `InFlight::mint` hands out only a free identifier and refuses only when the set is full.
- `record_head_accepts_only_a_head_the_controller_can_have_reached`: for any ring state and any
  head a controller reports, `SubmissionQueue::record_head` records the head only when it lies from
  the recorded head forward, round the ring, to the tail. Any other head returns
  `ImpossibleHeadError` with the ring unchanged, and no reported head panics the driver.
- `blocks_per_transfer_never_truncates_the_count`: for any transfer unit and any accepted LBA
  shift, `IdentifyNamespace::blocks_per_transfer` returns a count of at least 1 that covers exactly
  the unit, or `None`. A unit of 65536 logical blocks or more never wraps to a count of 0.

Each harness has a falsification record at `falsifications/<module.path>.<harness>.patch`.
Apply one with `patch -p1 < falsifications/<module.path>.<harness>.patch` from the crate root,
then run `cargo kani --harness <harness>`. The patch makes that harness fail; its header names the
assertion it expects to break. Kani may report a message it cannot print (it shows a formatted assertion
message as a placeholder), or an additional overflow check alongside the named assertion.
Undo the patch with `patch -R -p1 < <the same patch>`. The failure shows that the proof checks
something.

## Memory ordering

The crate performs no memory access, so the caller owns the ordering both rings need, and on a
weakly ordered host (Arm, RISC-V) it needs barriers. On the submission side, issue a write barrier
between the stores of the 64-byte entry and the write of the tail doorbell, or the controller may
fetch a stale entry. On the completion side, read dword 3 of the entry first and test its phase bit
(bit 16) against the phase `CompletionQueue::is_owned` expects. Once it matches, issue a read
barrier, then read dwords 0 to 2, and decode all four with `Completion::from_dwords`. Reading all
four dwords before the phase test lets the loads of dwords 0 to 2 run ahead of the controller's
write while dword 3 follows it, and the fresh phase then carries a stale head and command
identifier.

## BUGS

- `SubmissionQueue` models one ring and keeps no count of commands in flight. `push` returns
  `None` when the ring is full. The caller decides whether that is a bug or a reason to wait.
- `CompletionQueue::is_owned` tests the phase tag and nothing else. A controller that writes an
  entry with a fresh phase and a command identifier nobody issued passes it, so the caller matches
  `command_id` and `submission_queue_id` against the commands it has in flight.
- The queue builders `Command::create_io_completion_queue` and
  `Command::create_io_submission_queue` panic on fewer than two entries, and
  `physical_region_page_pair` panics on a page size that is not a power of two of at least 512
  bytes. These are caller mistakes, and no value the controller writes reaches either panic. The
  caller checks the queue depth against `Capabilities::maximum_queue_entries` and keeps queue
  identifier 0, the admin pair, out of the I/O queue builders.
- `physical_region_page_pair` covers at most two pages. A transfer that needs a PRP list returns
  `None`.
- `parse_identify_namespace` refuses LBA formats outside 512 bytes to 4 KiB per block.
  `logical_block_size_shift` reports the shift it refused.
- Not proven: the meaning of a status code within its code type, and anything in time, which
  belongs to the caller. The proofs place each part of the status field at its bits, and the crate
  names only the generic code type and two of its codes.
- Three cargo-fuzz targets in `fuzz/` feed the decoders arbitrary controller bytes.
  `identify_namespace` reads the Identify namespace structure and builds a transfer from it, for
  a 4096-byte unit and for a unit of any size. `completion` decodes completion queue entries,
  drives a completion ring with them and records each entry's submission queue head.
  `capabilities` decodes the `CAP` register and derives doorbell offsets and page pointers from it.
  Install the runner once with `cargo install cargo-fuzz`, then run one target with
  `cargo +nightly fuzz run <target>`; the weekly fuzz job runs all three for a bounded time.

The public structs that a later release may widen (`Completion`, `IdentifyController`,
`IdentifyNamespace` and `Namespace`) are `#[non_exhaustive]`. Build a `Namespace` from the Identify
answer with `Namespace::from_identify`, or with `Namespace::new` when there is no answer to read,
and read the other three from the controller's answers. Before Identify has answered, a driver holds
`Option<IdentifyNamespace>`; no placeholder value of the type stands for a namespace nobody has
read.

## How it differs from Linux and the Rust drivers

The Linux driver (`drivers/nvme/host`) owns the whole data path: a tag set per queue, PRP lists and
scatter-gather lists, many queue pairs, interrupts, reset, timeouts, shutdown and the admin command
set. The polled user-space driver of the Storage Performance Development Kit (SPDK) does the same
outside the kernel. Rust driver crates such as `nvme-driver` own their direct memory access and
issue the commands themselves, and Redox `nvmed` is a driver service of its own. This crate is none
of those. It builds one command at a time and walks one ring pair, as pure logic with no I/O, no
allocator, no `unsafe` and no dependency, so it compiles for any target and the model checker
reaches all of it. It checks what the controller hands back instead of trusting it: a submission
queue head the controller cannot have reached is an error, not a recorded value. So is a
completion for a command identifier `InFlight` does not hold, which stands in for the tag set.

What that costs you: you write the volatile register accessors, the direct memory access
allocation, the admin bring-up sequence and every wait, and you mint and release each command
identifier yourself, on push and on its completion. A transfer
longer than two pages has no PRP list to carry it. The crate gives the shutdown values
(`shutdown_controller_configuration` and the two `CONTROLLER_STATUS_SHUTDOWN_*` bits), and the
driver writes the one and waits for the other before it removes power.

## Minimum Rust version

Rust 1.85, the `rust-version` in `Cargo.toml`. To check it, run
`rustup toolchain install 1.85 --profile minimal`, then `cargo +1.85 test`. For the bare-metal
build, add the target with `rustup target add --toolchain 1.85 aarch64-unknown-none-softfloat` and
run `cargo +1.85 build --target aarch64-unknown-none-softfloat`.

## License

Licensed under either of MIT
([LICENSE-MIT](https://github.com/nifeos/non_volatile_memory_express/blob/main/LICENSE-MIT)) or
Apache-2.0
([LICENSE-APACHE](https://github.com/nifeos/non_volatile_memory_express/blob/main/LICENSE-APACHE)),
at your option.
