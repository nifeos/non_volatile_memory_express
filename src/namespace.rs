//! **The bounds check and transfer builder** for one namespace.

use crate::command::Command;
use crate::identify::IdentifyNamespace;
use crate::physical_region_page::physical_region_page_pair;

/// **Which way a transfer moves data**: from the device into host memory, or from host memory to
/// the device. [`Namespace::transfer_command`] takes one so that a call site says which command it
/// builds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// Device to host: the NVM Read command ([`Command::read`]).
    Read,
    /// Host to device: the NVM Write command ([`Command::write`]).
    Write,
}

/// **What a transfer needs to know about the namespace it addresses**: its capacity, the size of
/// the transfer unit the driver moves, and how many logical blocks one unit covers. A driver builds
/// it once from the Identify answer ([`Namespace::from_identify`]) and asks it whether a block
/// exists and for the command that moves one. The three facts are fixed together at construction,
/// so no call can pass a unit that disagrees with the block count the commands use.
///
/// # Examples
///
/// ```
/// use non_volatile_memory_express::{Direction, NON_VOLATILE_MEMORY_READ, Namespace};
///
/// // An 8 MiB namespace of 512-byte logical blocks holds 2048 blocks of 4096 bytes, numbered 0 to
/// // 2047, and each one is 8 logical blocks.
/// let namespace = Namespace::new(8 << 20, 4096, 8).unwrap();
/// assert!(namespace.holds_whole_block(2047));
/// assert!(!namespace.holds_whole_block(2048));
///
/// let command = namespace.transfer_command(7, 1, 2047, 0x4001_5000, 4096, Direction::Read);
/// let command = command.unwrap();
/// assert_eq!(command.0[0] & 0xff, NON_VOLATILE_MEMORY_READ as u32);
/// assert!(namespace.transfer_command(7, 1, 2048, 0x4001_5000, 4096, Direction::Read).is_none());
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct Namespace {
    capacity_bytes: u64,
    unit_bytes: u64,
    blocks_per_transfer: u16,
}

impl Namespace {
    /// A namespace of `capacity_bytes`, served in transfer units of `unit_bytes`, each covering
    /// `blocks_per_transfer` logical blocks, for a caller that has no Identify parse to hand.
    /// `None` when either count is zero, or when `unit_bytes` is not a whole multiple of
    /// `blocks_per_transfer`, since such a unit names no logical block size.
    pub fn new(capacity_bytes: u64, unit_bytes: u64, blocks_per_transfer: u16) -> Option<Self> {
        if unit_bytes == 0
            || blocks_per_transfer == 0
            || unit_bytes % u64::from(blocks_per_transfer) != 0
        {
            return None;
        }
        Some(Namespace {
            capacity_bytes,
            unit_bytes,
            blocks_per_transfer,
        })
    }

    /// The namespace Identify describes, served in transfer units of `unit_bytes`. `None` for a
    /// unit the namespace cannot serve ([`IdentifyNamespace::blocks_per_transfer`] refuses it) and
    /// for a capacity too large for a `u64` byte count.
    pub fn from_identify(identify: &IdentifyNamespace, unit_bytes: u64) -> Option<Self> {
        let blocks_per_transfer = identify.blocks_per_transfer(unit_bytes)?;
        let capacity_bytes = identify
            .blocks
            .checked_mul(1 << identify.logical_block_size_shift)?;
        Namespace::new(capacity_bytes, unit_bytes, blocks_per_transfer)
    }

    /// The namespace's capacity in bytes, the word [`IdentifyNamespace::capacity_bytes`] uses for
    /// the same quantity.
    pub fn capacity_bytes(&self) -> u64 {
        self.capacity_bytes
    }

    /// The bytes one transfer moves, which every block number on this value counts in.
    pub fn unit_bytes(&self) -> u64 {
        self.unit_bytes
    }

    /// The logical blocks one transfer unit covers.
    pub fn blocks_per_transfer(&self) -> u16 {
        self.blocks_per_transfer
    }

    /// **Is `block` a whole transfer unit inside this namespace?** A driver's own bounds check,
    /// run before a command is built rather than left to the controller's status field, because a
    /// driver that asks for an LBA past the end learns about it far from where it computed it.
    ///
    /// `false` for any `block` whose transfer would run past [`Namespace::capacity_bytes`], and for
    /// one whose end overflows a `u64`.
    pub fn holds_whole_block(&self, block: u64) -> bool {
        match block
            .checked_add(1)
            .and_then(|end_block| end_block.checked_mul(self.unit_bytes))
        {
            Some(end) => end <= self.capacity_bytes,
            None => false,
        }
    }

    /// **The NVM command for one whole-block transfer**, or `None` when the arithmetic will not
    /// close: a block outside the namespace ([`Namespace::holds_whole_block`]), an LBA that
    /// overflows, or a buffer that is not expressible as a PRP pair (which for a page-aligned
    /// buffer with [`Namespace::unit_bytes`] at most `2 * page_size_bytes` it always is; see
    /// [`physical_region_page_pair`]).
    ///
    /// `command_id` and `namespace_id` go to the command as they are, `data_physical_address` is
    /// the buffer's bus address, and `direction` picks [`Command::read`] or [`Command::write`].
    pub fn transfer_command(
        &self,
        command_id: u16,
        namespace_id: u32,
        block: u64,
        data_physical_address: u64,
        page_size_bytes: u64,
        direction: Direction,
    ) -> Option<Command> {
        if !self.holds_whole_block(block) {
            return None;
        }
        let starting_logical_block = block.checked_mul(u64::from(self.blocks_per_transfer))?;
        let pages =
            physical_region_page_pair(data_physical_address, self.unit_bytes, page_size_bytes)?;
        let blocks = self.blocks_per_transfer;
        let build = match direction {
            Direction::Read => Command::read,
            Direction::Write => Command::write,
        };
        Some(build(
            command_id,
            namespace_id,
            starting_logical_block,
            blocks,
            pages,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::{NON_VOLATILE_MEMORY_READ, NON_VOLATILE_MEMORY_WRITE};
    use crate::identify::parse_identify_namespace;

    /// The last block of the namespace is servable and the one after it is not, which is the
    /// check that keeps an out-of-range request from becoming a command the controller answers
    /// with a status the caller has to decode backwards.
    #[test]
    fn a_transfer_command_refuses_a_block_the_namespace_does_not_have() {
        const SIZE_BYTES: u64 = 8 * 1024 * 1024;
        const LAST: u64 = SIZE_BYTES / 4096 - 1;
        let namespace = Namespace::new(SIZE_BYTES, 4096, 8).unwrap();
        assert!(namespace.holds_whole_block(LAST));
        assert!(!namespace.holds_whole_block(LAST + 1));
        assert!(
            !namespace.holds_whole_block(u64::MAX),
            "no overflow, no wrap-around"
        );

        // And the command the last block produces addresses the LBA the geometry says it should.
        let command = namespace
            .transfer_command(7, 1, LAST, 0x4001_5000, 4096, Direction::Read)
            .expect("the last block is servable");
        assert_eq!(command.0[0] & 0xff, NON_VOLATILE_MEMORY_READ as u32);
        assert_eq!(
            command.0[10] as u64 | (command.0[11] as u64) << 32,
            LAST * 8
        );
        assert!(
            namespace
                .transfer_command(7, 1, LAST + 1, 0x4001_5000, 4096, Direction::Read)
                .is_none()
        );
    }

    /// `Direction::Write` builds the write opcode over the same block and pages a read would use,
    /// so the two directions differ in the opcode alone.
    #[test]
    fn the_direction_picks_the_opcode_and_nothing_else() {
        let namespace = Namespace::new(8 << 20, 4096, 8).unwrap();
        let read = namespace
            .transfer_command(7, 1, 3, 0x4001_5000, 4096, Direction::Read)
            .unwrap();
        let write = namespace
            .transfer_command(7, 1, 3, 0x4001_5000, 4096, Direction::Write)
            .unwrap();
        assert_eq!(read.0[0] & 0xff, NON_VOLATILE_MEMORY_READ as u32);
        assert_eq!(write.0[0] & 0xff, NON_VOLATILE_MEMORY_WRITE as u32);
        assert_eq!(read.0[1..], write.0[1..]);
        assert_eq!(read.0[0] >> 8, write.0[0] >> 8);
    }

    /// The constructor refuses the geometries that name no logical block size, and keeps the
    /// three facts it accepts.
    #[test]
    fn new_refuses_a_unit_that_names_no_logical_block_size() {
        assert!(Namespace::new(8 << 20, 0, 8).is_none());
        assert!(Namespace::new(8 << 20, 4096, 0).is_none());
        assert!(Namespace::new(8 << 20, 4096, 3).is_none());
        let namespace = Namespace::new(8 << 20, 4096, 8).unwrap();
        assert_eq!(namespace.capacity_bytes(), 8 << 20);
        assert_eq!(namespace.unit_bytes(), 4096);
        assert_eq!(namespace.blocks_per_transfer(), 8);
    }

    /// The Identify answer fixes the capacity and the block count; a unit that is not a whole
    /// number of logical blocks, and a capacity past `u64`, are refused rather than wrapped.
    #[test]
    fn from_identify_reads_the_geometry_and_refuses_what_it_cannot_serve() {
        let mut structure = [0u8; 4096];
        structure[0..8].copy_from_slice(&0x4000u64.to_le_bytes()); // 16384 blocks
        structure[130] = 9; // 512-byte logical blocks
        let identify = parse_identify_namespace(&structure).unwrap();
        let namespace = Namespace::from_identify(&identify, 4096).unwrap();
        assert_eq!(namespace.capacity_bytes(), 0x4000 * 512);
        assert_eq!(namespace.unit_bytes(), 4096);
        assert_eq!(namespace.blocks_per_transfer(), 8);
        assert!(Namespace::from_identify(&identify, 1000).is_none());

        structure[0..8].copy_from_slice(&(u64::MAX >> 8).to_le_bytes()); // past u64 in bytes
        let identify = parse_identify_namespace(&structure).unwrap();
        assert!(Namespace::from_identify(&identify, 4096).is_none());
        structure[0..8].copy_from_slice(&(u64::MAX >> 9).to_le_bytes()); // exactly fits
        let identify = parse_identify_namespace(&structure).unwrap();
        assert_eq!(
            Namespace::from_identify(&identify, 4096)
                .unwrap()
                .capacity_bytes(),
            (u64::MAX >> 9) << 9
        );
    }
}
