# The stranger cadence

The release's whole claim is that a stranger, given this repository and nothing else, can build
the crate and run its proofs from the README alone. A real agent runs that check on a
cadence, and the weekly `stranger cadence` job fails when the
last run is past 28 days.

What a run is:

1. Clone the repository fresh.
2. Follow the README and nothing else: build, test, and run the proofs.
3. Note every place the README lied, omitted, or assumed. Those are findings against the README,
   not against the stranger.
4. Record the date on the line below.

Last run: 2026-11-07

## What the last run did

A fresh clone, the README and nothing else, on a machine with no Rust toolchain.

- Installed Rust 1.88 with `rustup`, then ran `cargo build` and `cargo test`: 25 unit tests and 5
  documentation tests pass, one of them the README example.
- Installed Kani with `cargo install --locked kani-verifier`, ran `cargo kani setup`, then
  `cargo kani`: all eight harnesses verify.
- Applied each of the eight falsification patches, ran `cargo kani --harness <name>`, and undid the
  patch. Each harness failed with its patch applied, and the working tree was clean afterwards.

The README had six gaps, now filled: it gave no build or test command, no Kani version in the
install command, no way to run one harness, no redirect in the patch command and no revert
command, and it did not say that a falsified run may print a placeholder message for a formatted
assertion or an extra overflow check. Result: pass after these edits.
