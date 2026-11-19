//! The README's loop against a real controller: QEMU's emulated `NVMe` device, driven from Linux
//! user space with no kernel driver bound to it.
//!
//! It brings the controller up, reads Identify Controller and Identify namespace 1, creates one
//! I/O queue pair, reads block 0, writes block 1, flushes, reads block 1 back, reads block 0 until
//! both rings of the pair have wrapped twice more, and shuts the controller down cleanly. Block 0
//! is expected to start with `MARKER`, which the host writes into the disk image before it starts
//! QEMU, so a read that returns the wrong bytes fails. Every submission queue head the controller
//! reports must be the tail the example last wrote, since one command is in flight at a time.
//!
//! Two kernel command line words shape the run. `qemu_driver.depth=<entries>` sets the depth of
//! both ring pairs (64 by default; 2 is the smallest, and wraps every ring within the run).
//! `qemu_driver.page_bytes=<bytes>` sets the memory page size the controller is configured for
//! (4096 by default). A page larger than the host's 4 KiB page comes from a 2 MiB huge page, which
//! is physically contiguous, so process 1 reserves eight of them first.
//!
//! It refuses any controller but QEMU's (Peripheral Component Interconnect (PCI) vendor 1b36, device 0010) and any device a kernel
//! driver holds, so it cannot write to a real drive. It needs root, a kernel without an I/O memory
//! management unit in the path (physical and bus addresses are then the same), and `/proc` and
//! `/sys`. Run as process 1 it mounts those itself, writes to `/dev/console` and powers the
//! machine off at the end, so a static build of it can be the whole initial file system.
//! `docs/stranger-test.md` gives the commands that build that file system and run QEMU.
//!
//! Unlike the crate, this example uses `unsafe`: volatile access to the mapped registers and to
//! the memory the controller reads and writes, and five C library calls.

#[cfg(target_os = "linux")]
fn main() {
    std::process::exit(linux::main());
}

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("qemu_driver drives a PCI controller from Linux user space, and runs only on Linux");
}

#[cfg(target_os = "linux")]
mod linux {
    use core::ffi::{c_char, c_int, c_ulong, c_void};
    use non_volatile_memory_express::registers::{
        ADMIN_COMPLETION_QUEUE_BASE_PHYSICAL_ADDRESS_REGISTER, ADMIN_QUEUE_ATTRIBUTES_REGISTER,
        ADMIN_SUBMISSION_QUEUE_BASE_PHYSICAL_ADDRESS_REGISTER, CONTROLLER_CAPABILITIES_REGISTER,
        CONTROLLER_CONFIGURATION_REGISTER, CONTROLLER_STATUS_REGISTER,
    };
    use non_volatile_memory_express::{
        CONTROLLER_OR_NAMESPACE_STRUCTURE_CONTROLLER, CONTROLLER_OR_NAMESPACE_STRUCTURE_NAMESPACE,
        CONTROLLER_STATUS_FATAL, CONTROLLER_STATUS_READY, CONTROLLER_STATUS_SHUTDOWN_COMPLETE,
        Capabilities, Command, CommandIdentifierSet, Completion, CompletionQueue, Direction,
        Doorbell, Namespace, SubmissionQueue, admin_queue_attributes,
        controller_configuration_with_shutdown_notification, doorbell,
        enabled_controller_configuration, parse_identify_controller, parse_identify_namespace,
    };
    use std::fs::{self, File, OpenOptions};
    use std::io::{Read, Seek, SeekFrom, Write};
    use std::os::fd::AsRawFd;
    use std::sync::atomic::{Ordering, fence};
    use std::time::{Duration, Instant};

    /// What block 0 of the disk image starts with.
    const MARKER: &[u8] = b"non_volatile_memory_express QEMU run, block 0\n";
    /// What this example writes to block 1, repeated to fill the block.
    const WRITTEN: &[u8] = b"written by the qemu_driver example, block 1\n";
    /// The default memory page size, for the rings and the transfer buffers alike.
    const DEFAULT_PAGE_BYTES: u64 = 4096;
    /// By default each ring holds 64 entries: a submission ring of 64-byte entries then fills a
    /// 4 KiB page.
    const DEFAULT_DEPTH: u16 = 64;
    /// The namespace every QEMU `nvme` device without an `nvme-ns` device has.
    const NAMESPACE_ID: u32 = 1;
    /// How long one admin or I/O command may take before the example gives up.
    const COMMAND_TIMEOUT: Duration = Duration::from_secs(5);
    /// QEMU's PCI vendor and device identifiers for its `NVMe` controller.
    const QEMU_VENDOR_ID: &str = "0x1b36";
    const QEMU_CONTROLLER_DEVICE_ID: &str = "0x0010";

    const PROTECTION_READ_WRITE: c_int = 0x1 | 0x2;
    const MAP_SHARED: c_int = 0x01;
    const MAP_PRIVATE_ANONYMOUS_POPULATE: c_int = 0x02 | 0x20 | 0x8000;
    const MAP_HUGE_PAGE: c_int = 0x40000;
    /// The host's page, which `/proc/self/pagemap` indexes by.
    const HOST_PAGE_BYTES: u64 = 4096;
    /// The huge page a larger controller page comes from: one physically contiguous run.
    const HUGE_PAGE_BYTES: u64 = 2 << 20;
    /// How many huge pages process 1 reserves: six pages of memory for the controller, and spare.
    const HUGE_PAGES_RESERVED: &str = "8";
    /// The bytes one read or write moves: one transfer unit of the namespace, whatever the page.
    const TRANSFER_BYTES: u64 = 4096;
    const LINUX_REBOOT_POWER_OFF: c_int = 0x4321_fedc;

    unsafe extern "C" {
        fn mmap(
            address: *mut c_void,
            length: usize,
            protection: c_int,
            flags: c_int,
            file_descriptor: c_int,
            offset: i64,
        ) -> *mut c_void;
        fn mlock(address: *const c_void, length: usize) -> c_int;
        fn mount(
            source: *const c_char,
            target: *const c_char,
            file_system_type: *const c_char,
            flags: c_ulong,
            data: *const c_void,
        ) -> c_int;
        fn sync();
        fn reboot(command: c_int) -> c_int;
    }

    type Outcome<T> = Result<T, String>;

    /// The depth of each ring and the memory page size, from the kernel command line.
    #[derive(Clone, Copy)]
    struct Shape {
        depth: u16,
        page_bytes: u64,
    }

    impl Shape {
        fn from_command_line() -> Outcome<Self> {
            let line = fs::read_to_string("/proc/cmdline").unwrap_or_default();
            let mut shape = Shape {
                depth: DEFAULT_DEPTH,
                page_bytes: DEFAULT_PAGE_BYTES,
            };
            for word in line.split_whitespace() {
                if let Some(value) = word.strip_prefix("qemu_driver.depth=") {
                    shape.depth = value.parse().map_err(|_| format!("bad depth {value:?}"))?;
                } else if let Some(value) = word.strip_prefix("qemu_driver.page_bytes=") {
                    shape.page_bytes = value
                        .parse()
                        .map_err(|_| format!("bad page size {value:?}"))?;
                }
            }
            if shape.depth < 2 {
                return Err(format!(
                    "a ring holds at least 2 entries, not {}",
                    shape.depth
                ));
            }
            if u64::from(shape.depth) * 64 > shape.page_bytes {
                return Err(format!(
                    "a depth of {} does not fit one ring in a {}-byte page",
                    shape.depth, shape.page_bytes
                ));
            }
            if !shape.page_bytes.is_power_of_two()
                || !(HOST_PAGE_BYTES..=HUGE_PAGE_BYTES).contains(&shape.page_bytes)
            {
                return Err(format!("no memory for {}-byte pages", shape.page_bytes));
            }
            Ok(shape)
        }
    }

    /// Run the example, report on `out`, and power off when this is process 1.
    pub fn main() -> i32 {
        let is_initial_process = std::process::id() == 1;
        if is_initial_process {
            mount_kernel_file_systems();
            let _ = fs::write("/proc/sys/vm/nr_hugepages", HUGE_PAGES_RESERVED);
        }
        let mut out: Box<dyn Write> = match OpenOptions::new().write(true).open("/dev/console") {
            Ok(console) if is_initial_process => Box::new(console),
            _ => Box::new(std::io::stdout()),
        };
        let code = match run(&mut out) {
            Ok(()) => {
                let _ = writeln!(out, "qemu_driver: PASS");
                0
            }
            Err(reason) => {
                let _ = writeln!(out, "qemu_driver: FAIL: {reason}");
                1
            }
        };
        let _ = out.flush();
        if is_initial_process {
            // SAFETY: `sync` and `reboot` take no pointers; process 1 powering off is the end of
            // the run it was started for.
            unsafe {
                sync();
                reboot(LINUX_REBOOT_POWER_OFF);
            }
        }
        code
    }

    /// Process 1 starts with an empty root: mount `/proc` (for the page map), `/sys` (for the PCI
    /// device) and `/dev` (for the console).
    fn mount_kernel_file_systems() {
        for (target, file_system_type) in [
            (c"/proc", c"proc"),
            (c"/sys", c"sysfs"),
            (c"/dev", c"devtmpfs"),
        ] {
            let _ = fs::create_dir_all(target.to_str().unwrap_or("/"));
            // SAFETY: every pointer is a NUL-terminated literal that outlives the call, and a
            // null `data` is what these three file systems accept.
            unsafe {
                mount(
                    file_system_type.as_ptr(),
                    target.as_ptr(),
                    file_system_type.as_ptr(),
                    0,
                    core::ptr::null(),
                );
            }
        }
    }

    /// One memory page of the controller's size that it reads or writes, physically contiguous
    /// and locked in place, with its physical address read from `/proc/self/pagemap`.
    struct DirectMemoryPage {
        virtual_address: *mut u32,
        physical_address: u64,
        length_bytes: u64,
    }

    impl DirectMemoryPage {
        /// A host page for a 4 KiB controller page; a huge page, of which the controller uses
        /// the first `page_bytes`, for anything larger.
        fn new(page_bytes: u64) -> Outcome<Self> {
            let (mapped_bytes, flags) = if page_bytes == HOST_PAGE_BYTES {
                (HOST_PAGE_BYTES, MAP_PRIVATE_ANONYMOUS_POPULATE)
            } else {
                (
                    HUGE_PAGE_BYTES,
                    MAP_PRIVATE_ANONYMOUS_POPULATE | MAP_HUGE_PAGE,
                )
            };
            // SAFETY: an anonymous private mapping takes no file and aliases nothing.
            let mapped_virtual_address = unsafe {
                mmap(
                    core::ptr::null_mut(),
                    mapped_bytes as usize,
                    PROTECTION_READ_WRITE,
                    flags,
                    -1,
                    0,
                )
            };
            if mapped_virtual_address as isize == -1 {
                return Err(format!(
                    "mmap of {mapped_bytes} bytes for the controller failed (no huge pages?)"
                ));
            }
            // SAFETY: `mapped_virtual_address` is the mapping just made, `mapped_bytes` long.
            if unsafe { mlock(mapped_virtual_address, mapped_bytes as usize) } != 0 {
                return Err("mlock of a page for the controller failed".into());
            }
            let virtual_address = mapped_virtual_address.cast::<u32>();
            let physical_address = physical_address_of(virtual_address as u64)?;
            if physical_address % page_bytes != 0 {
                return Err(format!("page at {physical_address:#x} is not aligned"));
            }
            Ok(DirectMemoryPage {
                virtual_address,
                physical_address,
                length_bytes: page_bytes,
            })
        }

        fn read_u32(&self, byte_offset: usize) -> u32 {
            assert!(byte_offset + 4 <= self.length_bytes as usize && byte_offset % 4 == 0);
            // SAFETY: inside the page and aligned (asserted above); volatile because the
            // controller writes this memory behind the compiler's back.
            unsafe { core::ptr::read_volatile(self.virtual_address.add(byte_offset / 4)) }
        }

        fn write_u32(&self, byte_offset: usize, value: u32) {
            assert!(byte_offset + 4 <= self.length_bytes as usize && byte_offset % 4 == 0);
            // SAFETY: inside the page and aligned (asserted above); volatile because the
            // controller reads this memory.
            unsafe {
                core::ptr::write_volatile(self.virtual_address.add(byte_offset / 4), value);
            }
        }

        /// The first `TRANSFER_BYTES` of the page, which hold an Identify answer or one block.
        fn bytes(&self) -> Vec<u8> {
            (0..TRANSFER_BYTES as usize / 4)
                .flat_map(|word| self.read_u32(word * 4).to_le_bytes())
                .collect()
        }

        fn fill(&self, bytes: &[u8]) {
            for (word, chunk) in bytes
                .chunks(4)
                .enumerate()
                .take(TRANSFER_BYTES as usize / 4)
            {
                let mut value = [0u8; 4];
                value[..chunk.len()].copy_from_slice(chunk);
                self.write_u32(word * 4, u32::from_le_bytes(value));
            }
        }
    }

    /// The physical address behind one virtual address of this process: the page frame number
    /// is bits 0 to 54 of the page's 8-byte entry, valid when bit 63 (present) is set.
    fn physical_address_of(virtual_address: u64) -> Outcome<u64> {
        let mut page_map =
            File::open("/proc/self/pagemap").map_err(|error| format!("pagemap: {error}"))?;
        let mut entry = [0u8; 8];
        page_map
            .seek(SeekFrom::Start(virtual_address / HOST_PAGE_BYTES * 8))
            .and_then(|_| page_map.read_exact(&mut entry))
            .map_err(|error| format!("pagemap read: {error}"))?;
        let entry = u64::from_le_bytes(entry);
        let page_frame_number = entry & ((1 << 55) - 1);
        if entry >> 63 == 0 || page_frame_number == 0 {
            return Err("pagemap gives no frame (not root, or the page is not present)".into());
        }
        Ok(page_frame_number * HOST_PAGE_BYTES + virtual_address % HOST_PAGE_BYTES)
    }

    /// Base Address Register 0, mapped from the device's `resource0` file.
    struct Registers {
        base: *mut u32,
        length_bytes: u64,
    }

    impl Registers {
        fn map(device: &str) -> Outcome<Self> {
            let path = format!("{device}/resource0");
            let file = OpenOptions::new()
                .read(true)
                .write(true)
                .open(&path)
                .map_err(|error| format!("{path}: {error}"))?;
            let length_bytes = file
                .metadata()
                .map_err(|error| format!("{path}: {error}"))?
                .len();
            // SAFETY: a shared mapping of the whole register file, which stays valid after the
            // file closes; nothing else in this process maps it.
            let base = unsafe {
                mmap(
                    core::ptr::null_mut(),
                    length_bytes as usize,
                    PROTECTION_READ_WRITE,
                    MAP_SHARED,
                    file.as_raw_fd(),
                    0,
                )
            };
            if base as isize == -1 {
                return Err(format!("mmap of {path} failed"));
            }
            Ok(Registers {
                base: base.cast::<u32>(),
                length_bytes,
            })
        }

        fn read(&self, offset: u64) -> u32 {
            assert!(offset + 4 <= self.length_bytes && offset % 4 == 0);
            // SAFETY: inside the mapping and aligned (asserted above); a register read is volatile.
            unsafe { core::ptr::read_volatile(self.base.add(offset as usize / 4)) }
        }

        fn write(&self, offset: u64, value: u32) {
            assert!(offset + 4 <= self.length_bytes && offset % 4 == 0);
            // SAFETY: inside the mapping and aligned (asserted above); a register write is volatile.
            unsafe { core::ptr::write_volatile(self.base.add(offset as usize / 4), value) }
        }

        /// A 64-bit register as two 32-bit writes, low half first, which the specification allows.
        fn write_u64(&self, offset: u64, value: u64) {
            self.write(offset, value as u32);
            self.write(offset + 4, (value >> 32) as u32);
        }

        /// Wait until `ready(CSTS)` holds, or the controller reports a fatal status.
        fn wait_for_status(&self, deadline: Duration, ready: impl Fn(u32) -> bool) -> Outcome<()> {
            let started = Instant::now();
            loop {
                let status = self.read(CONTROLLER_STATUS_REGISTER);
                if ready(status) {
                    return Ok(());
                }
                if status & CONTROLLER_STATUS_FATAL != 0 {
                    return Err(format!("controller fatal status, CSTS {status:#x}"));
                }
                if started.elapsed() > deadline {
                    return Err(format!("timed out, CSTS {status:#x}"));
                }
                std::thread::sleep(Duration::from_millis(1));
            }
        }
    }

    /// One submission and completion ring pair, its memory, and its doorbells.
    struct QueuePair {
        queue_id: u16,
        submission_memory: DirectMemoryPage,
        completion_memory: DirectMemoryPage,
        submission: SubmissionQueue,
        completion: CompletionQueue,
        depth: u16,
        /// Every submission queue head the controller reported, in order.
        reported_heads: Vec<u16>,
    }

    impl QueuePair {
        fn new(queue_id: u16, shape: Shape) -> Outcome<Self> {
            Ok(QueuePair {
                queue_id,
                submission_memory: DirectMemoryPage::new(shape.page_bytes)?,
                completion_memory: DirectMemoryPage::new(shape.page_bytes)?,
                submission: SubmissionQueue::new(shape.depth),
                completion: CompletionQueue::new(shape.depth),
                depth: shape.depth,
                reported_heads: Vec::new(),
            })
        }

        /// How many times the completion ring has wrapped: one command completes per entry.
        fn laps(&self) -> usize {
            self.reported_heads.len() / usize::from(self.depth)
        }

        /// Submit the command `build` makes for a fresh identifier, poll for its completion as the
        /// README's memory ordering section says, and check what the controller hands back.
        fn execute(
            &mut self,
            registers: &Registers,
            stride: u32,
            identifiers: &mut CommandIdentifierSet<1>,
            build: impl FnOnce(u16) -> Outcome<Command>,
        ) -> Outcome<Completion> {
            let command_id = identifiers
                .acquire()
                .ok_or("no command identifier is free")?;
            let command = build(command_id)?;
            let slot = self
                .submission
                .push()
                .ok_or("the submission ring is full")?;
            for (index, dword) in command.0.iter().enumerate() {
                self.submission_memory
                    .write_u32(usize::from(slot) * 64 + index * 4, *dword);
            }
            fence(Ordering::SeqCst); // the entry before the tail doorbell
            let tail = u32::from(self.submission.tail());
            registers.write(
                doorbell(self.queue_id, Doorbell::SubmissionTail, stride),
                tail,
            );

            let entry = usize::from(self.completion.head()) * 16;
            let started = Instant::now();
            let dword_three = loop {
                let dword_three = self.completion_memory.read_u32(entry + 12);
                if Completion::is_phase_tag_set(dword_three)
                    == self.completion.expects_phase_tag_set()
                {
                    break dword_three;
                }
                if started.elapsed() > COMMAND_TIMEOUT {
                    return Err(format!(
                        "command {command_id} on queue {} timed out",
                        self.queue_id
                    ));
                }
                std::hint::spin_loop();
            };
            fence(Ordering::SeqCst); // dword 3 before dwords 0 to 2
            let done = Completion::from_dwords([
                self.completion_memory.read_u32(entry),
                self.completion_memory.read_u32(entry + 4),
                self.completion_memory.read_u32(entry + 8),
                dword_three,
            ]);
            if !self.completion.is_owned(&done) || done.submission_queue_id != self.queue_id {
                return Err(format!("a completion for another queue: {done:?}"));
            }
            identifiers
                .release(done.command_id)
                .map_err(|_| format!("a completion for no command in flight: {done:?}"))?;
            self.submission
                .record_head(done.submission_queue_head)
                .map_err(|error| format!("{error}"))?;
            self.reported_heads.push(done.submission_queue_head);
            if u32::from(done.submission_queue_head) != tail {
                return Err(format!(
                    "head {} after the one command in flight, tail {tail}",
                    done.submission_queue_head
                ));
            }
            let head = u32::from(self.completion.pop());
            registers.write(
                doorbell(self.queue_id, Doorbell::CompletionHead, stride),
                head,
            );
            if done.command_id != command_id || !done.is_success() {
                return Err(format!(
                    "command {command_id} failed: code type {}, code {:#x}",
                    done.status_code_type(),
                    done.status_code()
                ));
            }
            Ok(done)
        }
    }

    /// The QEMU `NVMe` device with no driver bound, as its `/sys/bus/pci/devices` directory.
    fn find_device() -> Outcome<String> {
        let read = |path: String| {
            fs::read_to_string(path)
                .unwrap_or_default()
                .trim()
                .to_owned()
        };
        let directory = fs::read_dir("/sys/bus/pci/devices").map_err(|error| format!("{error}"))?;
        for device in directory.flatten() {
            let path = device.path().display().to_string();
            if read(format!("{path}/class")) != "0x010802" {
                continue;
            }
            if read(format!("{path}/vendor")) != QEMU_VENDOR_ID
                || read(format!("{path}/device")) != QEMU_CONTROLLER_DEVICE_ID
            {
                return Err(format!(
                    "{path} is an NVMe controller but not QEMU's; refusing"
                ));
            }
            if fs::symlink_metadata(format!("{path}/driver")).is_ok() {
                return Err(format!("{path} has a kernel driver bound; refusing"));
            }
            return Ok(path);
        }
        Err("no NVMe controller on the PCI bus".into())
    }

    /// Set Memory Space Enable and Bus Master Enable in the PCI command register, so the
    /// controller answers its registers and may reach host memory.
    fn enable_bus_master(device: &str) -> Outcome<()> {
        let path = format!("{device}/config");
        let mut configuration = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .map_err(|error| format!("{path}: {error}"))?;
        let mut command = [0u8; 2];
        configuration
            .seek(SeekFrom::Start(4))
            .and_then(|_| configuration.read_exact(&mut command))
            .map_err(|error| format!("{path}: {error}"))?;
        let command = (u16::from_le_bytes(command) | 0b110).to_le_bytes();
        configuration
            .seek(SeekFrom::Start(4))
            .and_then(|_| configuration.write_all(&command))
            .map_err(|error| format!("{path}: {error}"))
    }

    /// Disable, program the admin rings, and enable: the bring-up the crate leaves to the caller.
    /// Returns the Controller Configuration value written, which shutdown starts from.
    fn bring_up(
        registers: &Registers,
        capabilities: Capabilities,
        admin: &QueuePair,
        shape: Shape,
    ) -> Outcome<u32> {
        let ready_timeout = Duration::from_millis(
            500 * u64::from(capabilities.timeout_in_500_millisecond_units().max(1)),
        );
        registers.write(CONTROLLER_CONFIGURATION_REGISTER, 0);
        registers
            .wait_for_status(ready_timeout, |status| {
                status & CONTROLLER_STATUS_READY == 0
            })
            .map_err(|error| format!("disable: {error}"))?;
        registers.write(
            ADMIN_QUEUE_ATTRIBUTES_REGISTER,
            admin_queue_attributes(shape.depth, shape.depth),
        );
        registers.write_u64(
            ADMIN_SUBMISSION_QUEUE_BASE_PHYSICAL_ADDRESS_REGISTER,
            admin.submission_memory.physical_address,
        );
        registers.write_u64(
            ADMIN_COMPLETION_QUEUE_BASE_PHYSICAL_ADDRESS_REGISTER,
            admin.completion_memory.physical_address,
        );
        let configuration = enabled_controller_configuration(shape.page_bytes)
            .ok_or("no configuration for the page")?;
        registers.write(CONTROLLER_CONFIGURATION_REGISTER, configuration);
        registers
            .wait_for_status(ready_timeout, |status| {
                status & CONTROLLER_STATUS_READY != 0
            })
            .map_err(|error| format!("enable: {error}"))?;
        Ok(configuration)
    }

    /// The whole run, from finding the device to its clean shutdown.
    fn run(out: &mut dyn Write) -> Outcome<()> {
        let shape = Shape::from_command_line()?;
        let device = find_device()?;
        enable_bus_master(&device)?;
        let registers = Registers::map(&device)?;
        let capabilities = Capabilities(
            u64::from(registers.read(CONTROLLER_CAPABILITIES_REGISTER))
                | u64::from(registers.read(CONTROLLER_CAPABILITIES_REGISTER + 4)) << 32,
        );
        let _ = writeln!(out, "qemu_driver: {device}, CAP {:#018x}", capabilities.0);
        if !(capabilities.minimum_page_size_bytes()..=capabilities.maximum_page_size_bytes())
            .contains(&shape.page_bytes)
            || capabilities.maximum_queue_entries() < u32::from(shape.depth)
        {
            return Err(format!(
                "the controller takes no {}-byte page or no ring of {} entries",
                shape.page_bytes, shape.depth
            ));
        }
        let stride = capabilities.doorbell_stride_exponent();
        let mut admin = QueuePair::new(0, shape)?;
        let configuration = bring_up(&registers, capabilities, &admin, shape)?;
        let _ = writeln!(
            out,
            "qemu_driver: enabled, CC {configuration:#010x}, {}-byte pages, rings of {}",
            shape.page_bytes, shape.depth
        );

        let mut identifiers = CommandIdentifierSet::<1>::new();
        let identify = DirectMemoryPage::new(shape.page_bytes)?;
        let identify_physical_address = identify.physical_address;
        admin.execute(&registers, stride, &mut identifiers, |command_id| {
            Ok(Command::identify(
                command_id,
                CONTROLLER_OR_NAMESPACE_STRUCTURE_CONTROLLER,
                0,
                identify_physical_address,
            ))
        })?;
        let controller =
            parse_identify_controller(&identify.bytes()).ok_or("Identify Controller")?;
        let _ = writeln!(
            out,
            "qemu_driver: namespace identifiers up to {}, transfers up to {} bytes",
            controller.maximum_namespace_id,
            controller.maximum_data_transfer_bytes(capabilities.minimum_page_size_bytes())
        );
        admin.execute(&registers, stride, &mut identifiers, |command_id| {
            Ok(Command::identify(
                command_id,
                CONTROLLER_OR_NAMESPACE_STRUCTURE_NAMESPACE,
                NAMESPACE_ID,
                identify_physical_address,
            ))
        })?;
        let identified = parse_identify_namespace(&identify.bytes()).ok_or("Identify namespace")?;
        let namespace =
            Namespace::from_identify(&identified, TRANSFER_BYTES).ok_or("the namespace")?;
        let _ = writeln!(
            out,
            "qemu_driver: namespace {NAMESPACE_ID}, {} blocks of {} bytes",
            identified.size_in_logical_blocks,
            1u64 << identified.logical_block_size_shift
        );

        let mut input_output = QueuePair::new(1, shape)?;
        let completion_physical_address = input_output.completion_memory.physical_address;
        let submission_physical_address = input_output.submission_memory.physical_address;
        admin.execute(&registers, stride, &mut identifiers, |command_id| {
            Ok(Command::create_io_completion_queue(
                command_id,
                1,
                shape.depth,
                completion_physical_address,
            ))
        })?;
        admin.execute(&registers, stride, &mut identifiers, |command_id| {
            Ok(Command::create_io_submission_queue(
                command_id,
                1,
                shape.depth,
                1,
                submission_physical_address,
            ))
        })?;
        transfers(
            out,
            &registers,
            stride,
            &mut input_output,
            &namespace,
            shape,
        )?;
        report_heads(out, "admin", &admin);
        report_heads(out, "I/O", &input_output);

        registers.write(
            CONTROLLER_CONFIGURATION_REGISTER,
            controller_configuration_with_shutdown_notification(configuration),
        );
        registers
            .wait_for_status(Duration::from_secs(10), |status| {
                status & CONTROLLER_STATUS_SHUTDOWN_COMPLETE != 0
            })
            .map_err(|error| format!("shutdown: {error}"))?;
        let _ = writeln!(out, "qemu_driver: shutdown complete");
        Ok(())
    }

    /// Read block 0 and check the marker, write block 1, flush, and read block 1 back.
    fn transfers(
        out: &mut dyn Write,
        registers: &Registers,
        stride: u32,
        queue: &mut QueuePair,
        namespace: &Namespace,
        shape: Shape,
    ) -> Outcome<()> {
        let mut identifiers = CommandIdentifierSet::<1>::new();
        let data = DirectMemoryPage::new(shape.page_bytes)?;
        let data_physical_address = data.physical_address;
        let transfer = |block, direction| {
            move |command_id| {
                namespace
                    .transfer_command(
                        command_id,
                        NAMESPACE_ID,
                        block,
                        data_physical_address,
                        shape.page_bytes,
                        direction,
                    )
                    .ok_or_else(|| format!("no transfer command for block {block}"))
            }
        };
        queue.execute(
            registers,
            stride,
            &mut identifiers,
            transfer(0, Direction::Read),
        )?;
        let block_zero = data.bytes();
        if !block_zero.starts_with(MARKER) {
            return Err(format!("block 0 starts {:?}", &block_zero[..MARKER.len()]));
        }
        let _ = writeln!(out, "qemu_driver: read block 0, the marker is there");

        let written: Vec<u8> = WRITTEN
            .iter()
            .copied()
            .cycle()
            .take(TRANSFER_BYTES as usize)
            .collect();
        data.fill(&written);
        queue.execute(
            registers,
            stride,
            &mut identifiers,
            transfer(1, Direction::Write),
        )?;
        queue.execute(registers, stride, &mut identifiers, |command_id| {
            Ok(Command::flush(command_id, NAMESPACE_ID))
        })?;
        data.fill(&[0; TRANSFER_BYTES as usize]);
        queue.execute(
            registers,
            stride,
            &mut identifiers,
            transfer(1, Direction::Read),
        )?;
        if data.bytes() != written {
            return Err("block 1 read back differs from what was written".into());
        }
        let _ = writeln!(
            out,
            "qemu_driver: wrote block 1, flushed, read it back unchanged"
        );

        // Two more laps of both rings, so a depth-2 run crosses each wrap several times.
        let laps_wanted = queue.laps() + 2;
        while queue.laps() < laps_wanted {
            data.fill(&[0; TRANSFER_BYTES as usize]);
            queue.execute(
                registers,
                stride,
                &mut identifiers,
                transfer(0, Direction::Read),
            )?;
            if !data.bytes().starts_with(MARKER) {
                return Err("a read of block 0 across a wrap returned other bytes".into());
            }
        }
        Ok(())
    }

    /// Print how far a ring pair ran: the commands, the laps, and up to 16 reported heads.
    fn report_heads(out: &mut dyn Write, which: &str, queue: &QueuePair) {
        let heads = &queue.reported_heads;
        let shown: Vec<String> = heads.iter().take(16).map(u16::to_string).collect();
        let more = if heads.len() > 16 { " ..." } else { "" };
        let _ = writeln!(
            out,
            "qemu_driver: {which} ring pair, {} commands, {} laps, every head the tail: {}{more}",
            heads.len(),
            queue.laps(),
            shown.join(" ")
        );
    }
}
