//! **The Controller Configuration values** that enable the controller for this driver and ask it to
//! shut down cleanly.

/// The Controller Configuration (`CC`) value that enables the controller with memory pages of
/// `page_size_bytes`: Enable (`EN`) = 1, I/O Command Set Selected (CSS) = 0 (the NVM command set),
/// Memory Page Size (MPS) = `log2(page_size_bytes) - 12`, Arbitration Mechanism Selected (AMS) = 0
/// (round-robin), I/O Submission Queue Entry Size (IOSQES) = 6 (64-byte submission entries), and
/// I/O Completion Queue Entry Size (IOCQES) = 4 (16-byte completion entries). The entry sizes are
/// the specification's only defined ones and the controller checks them before honoring a queue
/// creation, which is why they are set here rather than defaulted.
///
/// `None` for a page size that is not a power of two or lies outside the 4 KiB to 128 MiB the
/// four-bit field can name. The caller also keeps it between
/// [`crate::Capabilities::minimum_page_size_bytes`] and
/// [`crate::Capabilities::maximum_page_size_bytes`], and passes the same size to every
/// [`crate::physical_region_page_pair`] call, since the controller reads each pointer in pages of
/// this size.
///
/// # Examples
///
/// ```
/// use non_volatile_memory_express::enabled_controller_configuration;
///
/// assert_eq!(enabled_controller_configuration(4096), Some(0x0046_0001));
/// assert_eq!(enabled_controller_configuration(16384), Some(0x0046_0101)); // MPS = 2
/// assert_eq!(enabled_controller_configuration(2048), None); // below the smallest page
/// assert_eq!(enabled_controller_configuration(12288), None); // not a power of two
/// ```
#[doc(alias = "CC")]
#[doc(alias = "CC.EN")]
#[doc(alias = "MPS")]
#[doc(alias = "IOSQES")]
#[doc(alias = "IOCQES")]
pub fn enabled_controller_configuration(page_size_bytes: u64) -> Option<u32> {
    if !page_size_bytes.is_power_of_two() || !(1 << 12..=1 << 27).contains(&page_size_bytes) {
        return None;
    }
    let memory_page_size = page_size_bytes.trailing_zeros() - 12;
    Some(1 | memory_page_size << 7 | 6 << 16 | 4 << 20)
}

/// The configuration word's four fields share no bit (the enable at bit 0, MPS at 7:10, IOSQES at
/// 16:19, IOCQES at 20:23), which the compiler checks here; the mutation exclusion for
/// [`enabled_controller_configuration`] names this constant. The constant-assertion lint is off
/// on purpose: the assertion is constant by construction, and the compiler checking it is its job.
#[allow(clippy::assertions_on_constants, clippy::eq_op, dead_code)]
const CONTROLLER_CONFIGURATION_FIELDS_DO_NOT_OVERLAP: () =
    assert!(1 + (0xf << 7) + (6 << 16) + (4 << 20) == 0x0046_0781);

/// The Controller Configuration (`CC`) value that asks a running controller for a normal shutdown:
/// `configuration`, the value the driver last wrote, with Shutdown Notification (`SHN`, bits 14:15)
/// set to 01b and every other bit kept. Enable stays set; the specification asks the host to
/// notify the controller first and clear the enable bit only once Controller Status reports
/// [`crate::CONTROLLER_STATUS_SHUTDOWN_COMPLETE`], or after the shutdown time it allows has passed.
/// A controller with a volatile write cache flushes it before it reports completion, which is the
/// point of the notification: a drive powered off without it may lose what it acknowledged.
///
/// # Examples
///
/// ```
/// use non_volatile_memory_express::{
///     enabled_controller_configuration, shutdown_controller_configuration,
/// };
///
/// let running = enabled_controller_configuration(4096).unwrap();
/// assert_eq!(shutdown_controller_configuration(running), 0x0046_4001);
/// ```
#[doc(alias = "CC.SHN")]
#[doc(alias = "SHN")]
pub fn shutdown_controller_configuration(configuration: u32) -> u32 {
    (configuration & !(0b11 << 14)) | 0b01 << 14
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enabled_controller_configuration_sets_en_and_the_queue_entry_sizes() {
        let configuration = enabled_controller_configuration(4096).unwrap();
        assert_eq!(configuration & 1, 1, "EN");
        assert_eq!((configuration >> 7) & 0xf, 0, "MPS: 4 KiB pages, 2^(12+0)");
        assert_eq!(
            (configuration >> 16) & 0xf,
            6,
            "IOSQES: 64-byte submission entries, 2^6"
        );
        assert_eq!(
            (configuration >> 20) & 0xf,
            4,
            "IOCQES: 16-byte completion entries, 2^4"
        );
    }

    /// Every page size the field can name encodes as its exponent above 12, at bits 7:10, and the
    /// sizes either side of the range and between two powers of two are refused.
    #[test]
    fn the_page_size_encodes_as_its_exponent_and_nothing_else_is_accepted() {
        for memory_page_size in 0..=15u32 {
            let configuration = enabled_controller_configuration(1 << (12 + memory_page_size))
                .expect("a power of two from 4 KiB to 128 MiB");
            assert_eq!((configuration >> 7) & 0xf, memory_page_size);
            assert_eq!(configuration & !(0xf << 7), 0x0046_0001);
        }
        for refused in [0, 1, 2048, 4095, 4097, 12288, 1 << 28, u64::MAX] {
            assert_eq!(enabled_controller_configuration(refused), None, "{refused}");
        }
    }

    #[test]
    fn the_four_fields_share_no_bit_so_or_and_xor_agree() {
        // The `|` operators in `enabled_controller_configuration` survive being mutated to `^`,
        // and this is why: EN (bit 0), MPS (at most `0xf << 7`, bits 7-10), IOSQES (`6 << 16`,
        // bits 17-18) and IOCQES (`4 << 20`, bit 22) never share a bit, so `|` and `^` compute the
        // exact same `u32` for every page size the function accepts.
        let fields = [1u32, 0xf << 7, 6 << 16, 4 << 20];
        for (index, first) in fields.iter().enumerate() {
            for second in &fields[index + 1..] {
                assert_eq!(first & second, 0);
            }
        }
    }

    /// A normal shutdown notification writes 01b into bits 14:15 whatever they held, including an
    /// abrupt notification (10b) left by an earlier write, and keeps every other bit.
    #[test]
    fn the_shutdown_notification_is_01b_in_bits_14_and_15_and_nothing_else_moves() {
        for configuration in [
            0,
            0x0046_0001,
            0x0046_8001,
            0x0046_c001,
            u32::MAX,
            0x5555_5555,
        ] {
            let shutdown = shutdown_controller_configuration(configuration);
            assert_eq!((shutdown >> 14) & 0b11, 0b01, "{configuration:#x}");
            assert_eq!(shutdown & !(0b11 << 14), configuration & !(0b11 << 14));
        }
    }
}
