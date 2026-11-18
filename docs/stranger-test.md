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

Last run: 2026-11-18

## What the last run did

A fresh clone, the README and nothing else, on an x86-64 Linux host with Rust 1.99.1, Rust 1.85
and Kani 0.68.0 installed.

- `cargo build` and `cargo test` pass, the README example and the software controller example
  among the tests. `cargo run --example software_controller` prints the heads the README
  describes.
- `cargo kani` verifies all eighteen harnesses in about 75 seconds, compile included.
- Each of the eighteen falsification patches, applied as the README says, makes its harness fail
  under `cargo kani --harness <name>`, and the revert leaves the tree clean.
- `cargo +1.85 test` and `cargo +1.85 build --target aarch64-unknown-none-softfloat` pass.
- `cargo +nightly fuzz run completion -- -max_total_time=20` runs clean.
- The README driver, written from the README and the API documentation, runs against QEMU's
  `NVMe` controller (below) and passes.

The run found seven gaps, now filled:

- Six falsification patches applied only at an offset, so `patch` left `.orig` backups in `src/`.
  This run regenerated them against the current source.
- `fuzz/Cargo.lock` named an older crate version, so the first fuzz run rewrote it.
- The fuzz command gave no way to bound a run.
- The build and test commands sat under `## What is proven`, past the example they build.
- The README example called `Command::read` with five bare numbers and no word on which is which,
  and did not say that the third argument of `doorbell` is the raw `CAP.DSTRD` field.
- The README named no bring-up order: which registers to write, in which order, and what to wait
  for. A driver had to take it from the specification. `## Bring-up and shutdown` now gives it.
- The Kani timing claimed about 50 seconds; the run took about 75.

Result: pass after these edits.

## The QEMU run

`examples/qemu_driver.rs` is the README loop against QEMU's emulated `NVMe` controller. A static
build of it is the whole initial file system of a Linux guest, so no kernel driver ever binds the
controller, and the guest has no I/O memory management unit, so the physical address of a page is
its bus address. The disk is a scratch image file, the run touches no real drive, and the example
refuses any controller that is not QEMU's.

Build the example statically, and make a one-file initial file system of it:

```sh
RUSTFLAGS="-C target-feature=+crt-static" \
  cargo build --release --example qemu_driver --target x86_64-unknown-linux-gnu
mkdir -p run/root
cp target/x86_64-unknown-linux-gnu/release/examples/qemu_driver run/root/init
(cd run/root && echo init | cpio -o -H newc > ../initramfs.cpio)
```

Make a 64 MiB scratch disk whose block 0 starts with the marker the example checks:

```sh
qemu-img create -f raw run/disk.img 64M
printf 'non_volatile_memory_express QEMU run, block 0\n' | dd of=run/disk.img conv=notrunc
```

Any x86-64 Linux kernel with the Peripheral Component Interconnect (PCI) bus, the 8250 serial
console and `devtmpfs` built in will do; Ubuntu's `linux-image-unsigned` package has one
(`apt download`, then `dpkg-deb -x`). Boot it, bounded:

```sh
timeout 120 qemu-system-x86_64 -machine q35 -m 512 -nographic -no-reboot \
  -kernel vmlinuz -initrd run/initramfs.cpio -append "console=ttyS0 panic=-1 quiet" \
  -drive file=run/disk.img,if=none,id=disk,format=raw -device nvme,serial=scratch,drive=disk
dd if=run/disk.img bs=4096 skip=1 count=1 | head -1
```

Add `-accel kvm -cpu host` where the host allows it. The last run, QEMU 10.2.1 with an Ubuntu 7.0
kernel, printed:

```text
qemu_driver: /sys/bus/pci/devices/0000:00:03.0, CAP 0x004008200f0107ff
qemu_driver: enabled, CC 0x00460001
qemu_driver: namespace identifiers up to 256, transfers up to 524288 bytes
qemu_driver: namespace 1, 131072 blocks of 512 bytes
qemu_driver: read block 0, the marker is there
qemu_driver: wrote block 1, flushed, read it back unchanged
qemu_driver: shutdown complete
qemu_driver: PASS
```

and powered the guest off in about 3 seconds with the accelerator. The `dd` line printed
`written by the qemu_driver example, block 1`. The same run passed without the accelerator, and
with `logical_block_size=4096,physical_block_size=4096` on the device (16384 blocks of 4096
bytes). On a blank image it stopped at the read of block 0 with `FAIL`, as it should.
