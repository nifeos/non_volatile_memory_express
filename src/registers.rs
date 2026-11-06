//! **Register offsets in BAR0** (section 3.1). All are 4-byte registers or 8-byte registers the
//! caller accesses as two 4-byte halves (the specification permits either for the 64-bit ones).

/// Controller Capabilities (the register the specification calls `CAP`), 64-bit read-only.
/// See [`crate::Capabilities`].
#[doc(alias = "CAP")]
pub const CAPABILITIES: u64 = 0x00;
/// Controller Configuration (the register the specification calls `CC`), 32-bit. See
/// [`crate::enabled_controller_configuration`].
#[doc(alias = "CC")]
pub const CONTROLLER_CONFIGURATION_REGISTER: u64 = 0x14;
/// Controller Status (the register the specification calls `CSTS`), 32-bit read-only. See
/// [`crate::CONTROLLER_STATUS_READY`].
#[doc(alias = "CSTS")]
pub const CONTROLLER_STATUS_REGISTER: u64 = 0x1c;
/// Admin Queue Attributes (the register the specification calls `AQA`):
/// (completion queue entries - 1) << 16 | (submission queue entries - 1).
#[doc(alias = "AQA")]
pub const ADMIN_QUEUE_ATTRIBUTES_REGISTER: u64 = 0x24;
/// Admin Submission Queue Base Address (the register the specification calls `ASQ`), 64-bit,
/// page-aligned physical.
#[doc(alias = "ASQ")]
pub const ADMIN_SUBMISSION_QUEUE_BASE_ADDRESS_REGISTER: u64 = 0x28;
/// Admin Completion Queue Base Address (the register the specification calls `ACQ`), 64-bit,
/// page-aligned physical.
#[doc(alias = "ACQ")]
pub const ADMIN_COMPLETION_QUEUE_BASE_ADDRESS_REGISTER: u64 = 0x30;
/// The first doorbell. Doorbell N is at `DOORBELL_BASE + N * (4 << CAP.DSTRD)`; see
/// [`crate::doorbell`].
pub const DOORBELL_BASE: u64 = 0x1000;

/// Controller Status bit 0 (`CSTS.RDY`): the controller has come up (after the enable bit of
/// Controller Configuration is set) or gone down (after it is cleared).
#[doc(alias = "CSTS.RDY")]
#[doc(alias = "RDY")]
pub const CONTROLLER_STATUS_READY: u32 = 1;
/// Controller Status bit 1 (`CSTS.CFS`, Controller Fatal Status): the controller gave up, and no
/// doorbell will help.
#[doc(alias = "CSTS.CFS")]
#[doc(alias = "CFS")]
pub const CONTROLLER_STATUS_FATAL: u32 = 1 << 1;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fatal_bit_is_a_different_bit_from_the_ready_bit() {
        // Nothing in this crate reads Controller Status; the caller's volatile layer does. This
        // transcription check is the only place on the host that would catch a constant copied to
        // the wrong bit (the fatal bit is bit 1, the ready bit is bit 0).
        assert_eq!(CONTROLLER_STATUS_FATAL, 0b10);
        assert_ne!(CONTROLLER_STATUS_FATAL, CONTROLLER_STATUS_READY);
    }
}
