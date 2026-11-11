//! **The Controller Configuration value** that enables the controller for this driver.

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
}
