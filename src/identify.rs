//! **Identify parsing**: the facts a block driver needs from the controller's answers about a
//! namespace and about itself, read defensively.

/// The two facts the driver needs from the 4096-byte Identify namespace structure (section 5.15.2,
/// figure 245): the namespace size in logical blocks (Namespace Size (`NSZE`), bytes 0..8) and the
/// data shift of the logical block address (LBA) format in use (Formatted LBA Size (`FLBAS`) names
/// one of 16 formats; each format's LBA Data Size (`LBADS`) is `1 << shift` bytes per block). `None`
/// if the structure is shorter than the fields it must carry, or describes a block size outside
/// what a driver of 4096-byte blocks can serve (`LBADS` below 9 is the specification's "unsupported"
/// marker; above 12 a block would outgrow the transfer buffer).
pub fn parse_identify_namespace(data: &[u8]) -> Option<IdentifyNamespace> {
    if data.len() < 384 {
        return None; // NSZE..the LBA format table must be present
    }
    let namespace_size = u64::from_le_bytes(data[0..8].try_into().ok()?);
    let format_index = data[26] & 0xf;
    // LBA format `format_index` is 4 bytes at 128 + 4*format_index; LBADS is its third byte.
    let data_shift = data[128 + 4 * format_index as usize + 2];
    if !(9..=12).contains(&data_shift) {
        return None;
    }
    Some(IdentifyNamespace {
        blocks: namespace_size,
        logical_block_size_shift: data_shift as u32,
    })
}

/// **The LBA data shift the namespace is formatted with, whatever it is**: the same `LBADS`
/// (LBA Data Size) byte [`parse_identify_namespace`] reads, before that function's bounds refuse
/// it. `None` only when the structure is too short to carry the format table.
///
/// It exists so a refusal can say what it refused. A drive must be formatted so that
/// `blocks_per_transfer` falls in `1..=8`, and a namespace this driver declines has to print its
/// LBA size rather than arrive as a server that never started and a test that reads "skipped".
pub fn logical_block_size_shift(data: &[u8]) -> Option<u8> {
    if data.len() < 384 {
        return None;
    }
    let format_index = data[26] & 0xf;
    Some(data[128 + 4 * format_index as usize + 2])
}

/// See [`parse_identify_namespace`]. The struct is `#[non_exhaustive]`: a later release may read
/// more of the structure, so code outside the crate gets one only from the parse. A driver holds
/// `Option<IdentifyNamespace>` for the time before Identify has answered, so no value of this type
/// stands for a namespace nobody has read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct IdentifyNamespace {
    /// The namespace's size in logical blocks, the Namespace Size (`NSZE`).
    #[doc(alias = "NSZE")]
    pub blocks: u64,
    /// `1 << logical_block_size_shift` bytes per logical block; between 9 and 12 by construction.
    /// The specification calls this exponent the LBA Data Size (`LBADS`) of the format that
    /// Formatted LBA Size (`FLBAS`) selects.
    #[doc(alias = "LBADS")]
    #[doc(alias = "FLBAS")]
    pub logical_block_size_shift: u32,
}

impl IdentifyNamespace {
    /// The namespace's capacity in bytes.
    pub fn capacity_bytes(&self) -> u64 {
        self.blocks << self.logical_block_size_shift
    }
    /// How many logical blocks one `unit_bytes`-byte transfer covers, always at least 1. `None` if
    /// `unit_bytes` is zero, is not a whole number of logical blocks (a namespace this driver
    /// cannot serve 4096-byte blocks on; `parse_identify_namespace`'s shift bounds make it
    /// impossible for a unit of 4096 bytes), or covers more than the 65535 logical blocks a `u16`
    /// count holds (a unit of 65536 blocks or more, which would otherwise wrap to a count of 0 that
    /// [`crate::Command::read`] and [`crate::Command::write`] refuse).
    ///
    /// # Examples
    ///
    /// ```
    /// use non_volatile_memory_express::{IdentifyNamespace, parse_identify_namespace};
    ///
    /// let mut structure = [0u8; 4096];
    /// structure[130] = 9; // format 0, the one in use, has 512-byte logical blocks
    /// let namespace: IdentifyNamespace = parse_identify_namespace(&structure).unwrap();
    /// assert_eq!(namespace.blocks_per_transfer(4096), Some(8));
    /// assert_eq!(namespace.blocks_per_transfer(65_535 * 512), Some(65_535));
    /// assert_eq!(namespace.blocks_per_transfer(65_536 * 512), None); // too many to count
    /// assert_eq!(namespace.blocks_per_transfer(1000), None); // not a whole number of blocks
    /// ```
    pub fn blocks_per_transfer(&self, unit_bytes: u64) -> Option<u16> {
        if unit_bytes == 0 || unit_bytes & ((1 << self.logical_block_size_shift) - 1) != 0 {
            return None;
        }
        u16::try_from(unit_bytes >> self.logical_block_size_shift).ok()
    }
}

/// The two facts a driver needs from the 4096-byte Identify Controller structure (the one
/// [`crate::CONTROLLER_OR_NAMESPACE_STRUCTURE_CONTROLLER`] selects): the Maximum Data Transfer Size (`MDTS`, byte 77) and the Number of Namespaces (`NN`, bytes 516
/// to 519). `None` only if the structure is shorter than the 520 bytes that reach the last of
/// them; every value the controller may write in them is accepted, since each has a meaning.
///
/// # Examples
///
/// ```
/// use non_volatile_memory_express::parse_identify_controller;
///
/// let mut structure = [0u8; 4096];
/// structure[77] = 5; // MDTS: 2^5 minimum pages per transfer
/// structure[516..520].copy_from_slice(&4u32.to_le_bytes()); // NN: namespaces 1 to 4
/// let controller = parse_identify_controller(&structure).unwrap();
/// assert_eq!(controller.maximum_data_transfer_bytes(4096), 128 * 1024);
/// assert_eq!(controller.maximum_namespace_id, 4);
/// ```
pub fn parse_identify_controller(data: &[u8]) -> Option<IdentifyController> {
    let number_of_namespaces = data.get(516..520)?;
    Some(IdentifyController {
        maximum_data_transfer_size_exponent: data[77],
        maximum_namespace_id: u32::from_le_bytes(number_of_namespaces.try_into().ok()?),
    })
}

/// See [`parse_identify_controller`]. The struct is `#[non_exhaustive]`: a later release may read
/// more of the structure, so code outside the crate gets one only from the parse.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct IdentifyController {
    /// The Maximum Data Transfer Size (`MDTS`) as the controller wrote it: a transfer may span at
    /// most `2^maximum_data_transfer_size_exponent` pages of the controller's minimum page size,
    /// and 0 means the controller states no limit. Read it in bytes with
    /// [`IdentifyController::maximum_data_transfer_bytes`].
    #[doc(alias = "MDTS")]
    pub maximum_data_transfer_size_exponent: u8,
    /// The Number of Namespaces (`NN`): the largest namespace identifier valid on this controller.
    /// Not every identifier from 1 to it need be active; the
    /// [`crate::CONTROLLER_OR_NAMESPACE_STRUCTURE_ACTIVE_NAMESPACE_LIST`] selector lists the ones
    /// that are.
    #[doc(alias = "NN")]
    pub maximum_namespace_id: u32,
}

impl IdentifyController {
    /// The largest transfer one command may carry, in bytes, given the controller's minimum page
    /// size ([`crate::Capabilities::minimum_page_size_bytes`]). `u64::MAX` when the controller
    /// states no limit, and also when the limit is too large for a `u64`, since no transfer can
    /// reach it either way. A driver keeps every transfer at or below it, whatever the page pair
    /// or list could otherwise describe.
    #[doc(alias = "MDTS")]
    pub fn maximum_data_transfer_bytes(&self, minimum_page_size_bytes: u64) -> u64 {
        match self.maximum_data_transfer_size_exponent {
            0 => u64::MAX,
            exponent => 1u64.checked_shl(exponent.into()).map_or(u64::MAX, |pages| {
                pages.saturating_mul(minimum_page_size_bytes)
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identify_namespace_yields_size_and_shift() {
        let mut data = [0u8; 4096];
        data[0..8].copy_from_slice(&0x4000u64.to_le_bytes()); // NSZE: 16384 blocks
        data[26] = 0; // FLBAS: format 0
        data[130] = 9; // format 0's LBADS: 512-byte blocks
        let id = parse_identify_namespace(&data).unwrap();
        assert_eq!(id.blocks, 0x4000);
        assert_eq!(id.logical_block_size_shift, 9);
        assert_eq!(id.capacity_bytes(), 0x4000 * 512);
        assert_eq!(id.blocks_per_transfer(4096), Some(8));

        data[130] = 13; // an LBA format this driver cannot serve
        assert!(parse_identify_namespace(&data).is_none());
        assert!(parse_identify_namespace(&[0u8; 100]).is_none(), "truncated");
    }

    /// A refused format still says what it was, which is what lets a caller print "8192-byte
    /// LBAs" instead of "skipped". Read through `FLBAS`, so a namespace formatted with its second
    /// format reports that one rather than format 0.
    #[test]
    fn a_refused_lba_format_still_reports_its_shift() {
        let mut data = [0u8; 4096];
        data[26] = 1; // FLBAS: format 1
        data[130] = 9; // format 0: 512 bytes, not the one in use
        data[134] = 13; // format 1: 8192 bytes
        assert!(parse_identify_namespace(&data).is_none());
        assert_eq!(logical_block_size_shift(&data), Some(13));
        assert_eq!(logical_block_size_shift(&[0u8; 383]), None);
    }

    /// The shift is read at `128 + 4 * FLBAS + 2` for every format the nibble can name, from a
    /// structure exactly 384 bytes long, with a different shift planted in every slot so a wrong
    /// stride lands on another format's value.
    #[test]
    fn the_reported_shift_is_the_one_in_the_format_index_slot_for_every_format() {
        for format_index in 0..16u8 {
            let mut data = [0u8; 384];
            for format in 0..16usize {
                data[128 + 4 * format + 2] = 20 + format as u8;
            }
            data[26] = format_index | 0x70; // the bits above the format number are not part of it
            assert_eq!(
                logical_block_size_shift(&data),
                Some(20 + format_index),
                "format_index {format_index}"
            );
        }
        assert_eq!(
            logical_block_size_shift(&[0u8; 384]),
            Some(0),
            "384 bytes is enough"
        );
    }

    #[test]
    fn identify_namespace_accepts_the_minimum_length_and_refuses_one_byte_less() {
        // The `[0u8; 100]` case above is far short of the boundary and cannot tell `< 384` from
        // `<= 384`. Sit exactly on it instead.
        let mut data = [0u8; 384];
        data[0..8].copy_from_slice(&1u64.to_le_bytes());
        data[26] = 0;
        data[130] = 9;
        assert!(
            parse_identify_namespace(&data).is_some(),
            "384 bytes is documented as enough"
        );
        assert!(
            parse_identify_namespace(&data[..383]).is_none(),
            "one byte short of the documented minimum must be refused"
        );
    }

    #[test]
    fn identify_namespace_reads_the_logical_block_format_table_at_the_format_index_not_format_zero()
    {
        // The test above uses FLBAS=0, where the table offset `128 + 4*format_index` happens to equal
        // `128 - 4*format_index`: both are 128. Use FLBAS=1 so the two diverge, and plant a different,
        // still-valid LBADS at format 0's slot so a parser that reads the wrong slot is caught by
        // value rather than by accidentally landing on unset (and therefore refused) bytes.
        let mut data = [0u8; 384];
        data[0..8].copy_from_slice(&1u64.to_le_bytes());
        data[26] = 1; // FLBAS: format 1
        data[128 + 2] = 10; // format 0's LBADS: a decoy
        data[128 + 4 + 2] = 11; // format 1's LBADS: the one that must be read
        let id = parse_identify_namespace(&data).unwrap();
        assert_eq!(
            id.logical_block_size_shift, 11,
            "format 1's slot, not format 0's"
        );
    }

    #[test]
    fn blocks_per_transfer_masks_by_the_actual_block_size() {
        // `identify_namespace_yields_size_and_shift`'s only `blocks_per_transfer` call is 4096, which
        // is a multiple of every mask this function could plausibly compute (511, 512, or 513: none
        // of their low bits reach as high as bit 12), so it cannot tell them apart, and it never
        // calls `blocks_per_transfer(0)` either.
        let id = IdentifyNamespace {
            blocks: 1,
            logical_block_size_shift: 9, // 512-byte blocks; mask is 511
        };
        assert_eq!(
            id.blocks_per_transfer(0),
            None,
            "a zero unit is refused even though 0 & anything is 0"
        );
        // 1536 = 3 * 512, so the correct mask (511) says it divides evenly. The `+1` mutant's
        // mask (513) and the `/1` mutant's mask (512) both share bit 9 with 1536 and would
        // wrongly refuse it.
        assert_eq!(id.blocks_per_transfer(1536), Some(3));
    }

    /// The count is a `u16`, so a unit of 65536 logical blocks or more is refused rather than
    /// truncated: 65536 would wrap to 0, which no command can carry, and 65537 to 1, which would
    /// move one block where the caller asked for a great many. Checked at every shift a parse
    /// accepts, right on either side of the limit.
    #[test]
    fn blocks_per_transfer_refuses_a_unit_too_large_to_count_rather_than_wrapping() {
        for shift in 9..=12u32 {
            let id = IdentifyNamespace {
                blocks: 1,
                logical_block_size_shift: shift,
            };
            let block = 1u64 << shift;
            assert_eq!(
                id.blocks_per_transfer(u64::from(u16::MAX) * block),
                Some(u16::MAX)
            );
            assert_eq!(
                id.blocks_per_transfer(65_536 * block),
                None,
                "shift {shift}"
            );
            assert_eq!(
                id.blocks_per_transfer(65_537 * block),
                None,
                "shift {shift}"
            );
            assert_eq!(id.blocks_per_transfer(!(block - 1)), None);
        }
    }

    #[test]
    fn identify_controller_reads_the_transfer_limit_and_the_namespace_count() {
        let mut data = [0u8; 4096];
        data[76] = 0xff; // the byte before MDTS, which must not leak into it
        data[77] = 3;
        data[78] = 0xff; // and the byte after
        data[515] = 0xff;
        data[516..520].copy_from_slice(&0x0102_0304u32.to_le_bytes());
        data[520] = 0xff;
        let controller = parse_identify_controller(&data).unwrap();
        assert_eq!(controller.maximum_data_transfer_size_exponent, 3);
        assert_eq!(controller.maximum_namespace_id, 0x0102_0304);
        assert_eq!(parse_identify_controller(&data[..520]), Some(controller));
        assert_eq!(parse_identify_controller(&data[..519]), None, "truncated");
        assert_eq!(parse_identify_controller(&[]), None);
    }

    /// `MDTS` counts minimum pages as a power of two; 0 states no limit, and a limit beyond a
    /// `u64` is no limit either.
    #[test]
    fn the_transfer_limit_is_a_power_of_two_of_minimum_pages_and_0_is_unlimited() {
        let controller = |exponent| IdentifyController {
            maximum_data_transfer_size_exponent: exponent,
            maximum_namespace_id: 1,
        };
        assert_eq!(controller(0).maximum_data_transfer_bytes(4096), u64::MAX);
        assert_eq!(controller(1).maximum_data_transfer_bytes(4096), 8192);
        assert_eq!(controller(5).maximum_data_transfer_bytes(4096), 131_072);
        assert_eq!(controller(5).maximum_data_transfer_bytes(16384), 524_288);
        assert_eq!(controller(51).maximum_data_transfer_bytes(4096), 1 << 63);
        assert_eq!(controller(52).maximum_data_transfer_bytes(4096), u64::MAX);
        assert_eq!(controller(63).maximum_data_transfer_bytes(1), 1 << 63);
        assert_eq!(controller(64).maximum_data_transfer_bytes(1), u64::MAX);
        assert_eq!(controller(255).maximum_data_transfer_bytes(4096), u64::MAX);
    }
}
