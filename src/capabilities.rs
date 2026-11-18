//! **The decoded Controller Capabilities register**: the fields of `CAP` a driver acts on.

/// The decoded fields of `CAP` this driver acts on. A newtype over the raw register value so the
/// bit positions live in exactly one place.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capabilities(pub u64);

impl Capabilities {
    /// Maximum Queue Entries Supported (`CAP.MQES`), converted from the register's 0-based field
    /// to a count. Every queue this driver creates must be no deeper than this.
    #[doc(alias = "CAP.MQES")]
    #[doc(alias = "MQES")]
    pub fn maximum_queue_entries(self) -> u32 {
        (self.0 as u32 & 0xffff) + 1
    }
    /// Doorbell Stride (`CAP.DSTRD`), the raw 4-bit exponent field, not a byte count. The stride in
    /// bytes is `2^(2 + value)`, so the value is at most 15 and the stride at most `4 << 15` bytes
    /// (a shift of 17); doorbell N is at `0x1000 + N * (4 << value)`, and the multiply cannot
    /// overflow a u64 for any real doorbell index. Base Specification 2.4, `CAP`
    /// (section 3.1.4) and the doorbell register offsets (section 8.2.3).
    #[doc(alias = "CAP.DSTRD")]
    #[doc(alias = "DSTRD")]
    pub fn doorbell_stride_exponent(self) -> u32 {
        (self.0 >> 32) as u32 & 0xf
    }
    /// Timeout (`CAP.TO`) for the ready bit to follow the enable bit, in units of 500 milliseconds.
    #[doc(alias = "CAP.TO")]
    #[doc(alias = "TO")]
    pub fn timeout_in_500_millisecond_units(self) -> u32 {
        (self.0 >> 24) as u32 & 0xff
    }
    /// Memory Page Size Minimum (`CAP.MPSMIN`), as a byte count (the field is `2^(12+MPSMIN)`).
    /// The page size a driver passes to [`crate::enabled_controller_configuration`] is at least
    /// this.
    #[doc(alias = "CAP.MPSMIN")]
    #[doc(alias = "MPSMIN")]
    pub fn minimum_page_size_bytes(self) -> u64 {
        1u64 << (12 + ((self.0 >> 48) & 0xf))
    }
    /// Memory Page Size Maximum (`CAP.MPSMAX`), as a byte count (the field is `2^(12+MPSMAX)`).
    /// The page size a driver passes to [`crate::enabled_controller_configuration`] is at most
    /// this.
    #[doc(alias = "CAP.MPSMAX")]
    #[doc(alias = "MPSMAX")]
    pub fn maximum_page_size_bytes(self) -> u64 {
        1u64 << (12 + ((self.0 >> 52) & 0xf))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capabilities_fields_decode_from_their_specified_positions() {
        // A default QEMU nvme device reports a value in this family: MQES=0x7ff, TO=0xf, DSTRD=0,
        // MPSMIN=0. Build one by hand and read every field back.
        let capabilities = Capabilities(0x7ffu64 | (0xf << 24) | (2 << 32) | (1 << 48) | (3 << 52));
        assert_eq!(capabilities.maximum_queue_entries(), 0x800);
        assert_eq!(capabilities.timeout_in_500_millisecond_units(), 0xf);
        assert_eq!(capabilities.doorbell_stride_exponent(), 2);
        assert_eq!(capabilities.minimum_page_size_bytes(), 8192);
        assert_eq!(capabilities.maximum_page_size_bytes(), 32768);
    }
}
