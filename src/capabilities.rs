//! **The decoded Controller Capabilities register**: the fields of `CAP` a driver acts on.

/// The decoded fields of `CAP` this driver acts on. A newtype over the raw register value so the
/// bit positions live in exactly one place.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capabilities(pub u64);

impl Capabilities {
    /// Maximum Queue Entries Supported, converted from the register's 0-based field to a count.
    /// Every queue this driver creates must be no deeper than this.
    pub fn maximum_queue_entries(self) -> u32 {
        (self.0 as u32 & 0xffff) + 1
    }
    /// Doorbell Stride: doorbell N is at `0x1000 + N * (4 << DSTRD)` (a 4-bit field, so the shift
    /// is at most 19 and the multiply cannot overflow a u64 for any real doorbell index).
    pub fn doorbell_stride(self) -> u32 {
        (self.0 >> 32) as u32 & 0xf
    }
    /// Timeout for the ready bit to follow the enable bit, in units of 500 milliseconds.
    #[doc(alias = "CAP.TO")]
    #[doc(alias = "TO")]
    pub fn timeout_in_500_millisecond_units(self) -> u32 {
        (self.0 >> 24) as u32 & 0xff
    }
    /// Memory Page Size Minimum, as a byte count (the field is `2^(12+MPSMIN)`). This driver runs
    /// with `CC.MPS` = 0, i.e. 4 KiB pages, so bring-up checks this is at most 4096.
    pub fn minimum_page_size(self) -> u64 {
        1u64 << (12 + ((self.0 >> 48) & 0xf))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capabilities_fields_decode_from_their_specified_positions() {
        // A default QEMU nvme device reports a value in this family: MQES=0x7ff, TO=0xf, DSTRD=0,
        // MPSMIN=0. Build one by hand and read every field back.
        let capabilities = Capabilities(0x7ffu64 | (0xf << 24) | (2 << 32) | (1 << 48));
        assert_eq!(capabilities.maximum_queue_entries(), 0x800);
        assert_eq!(capabilities.timeout_in_500_millisecond_units(), 0xf);
        assert_eq!(capabilities.doorbell_stride(), 2);
        assert_eq!(capabilities.minimum_page_size(), 8192);
    }
}
