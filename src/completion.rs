//! **Completion queue entries**: the four dwords the controller writes, decoded.

/// One 16-byte completion queue entry, decoded from the four dwords the controller wrote.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct Completion {
    /// Command-specific result (dword 0, the specification's `DW0`).
    #[doc(alias = "DW0")]
    pub result: u32,
    /// The submission queue head (`SQHD`, dword 2 bits 0 to 15) as the controller last consumed
    /// it: how the driver learns its submission ring entries are reusable.
    #[doc(alias = "SQHD")]
    pub submission_queue_head: u16,
    /// Which submission queue the completed command came from (`SQID`, dword 2 bits 16 to 31).
    #[doc(alias = "SQID")]
    pub submission_queue_id: u16,
    /// The command identifier (`CID`, dword 3 bits 0 to 15) the driver assigned, echoed back.
    #[doc(alias = "CID")]
    pub command_id: u16,
    /// The phase tag (`P`): dword 3, bit 16. See [`crate::CompletionQueue::is_owned`].
    #[doc(alias = "P")]
    pub phase: bool,
    /// The 15-bit status field (`SF`), dword 3 bits 17 to 31; 0 is success. The methods
    /// [`Completion::is_success`], [`Completion::status_code`], [`Completion::status_code_type`],
    /// [`Completion::command_retry_delay`], [`Completion::is_more_set`] and
    /// [`Completion::is_do_not_retry_set`] read its parts.
    #[doc(alias = "SF")]
    pub status: u16,
}

/// The Generic Command Status code type (`SCT` 0h): the status codes every command set shares.
#[doc(alias = "SCT")]
pub const STATUS_CODE_TYPE_GENERIC: u8 = 0x0;
/// Generic status code 00h, Successful Completion: the command did what it was asked.
#[doc(alias = "SC")]
pub const GENERIC_STATUS_SUCCESSFUL_COMPLETION: u8 = 0x00;
/// Generic status code 02h, Invalid Field in Command: the controller refused a field of the
/// command, such as an unsupported selector or a queue size beyond its maximum.
#[doc(alias = "SC")]
pub const GENERIC_STATUS_INVALID_FIELD_IN_COMMAND: u8 = 0x02;

impl Completion {
    /// Decode the four dwords the controller wrote into a completion queue slot.
    pub fn from_dwords(dwords: [u32; 4]) -> Self {
        Completion {
            result: dwords[0],
            submission_queue_head: dwords[2] as u16,
            submission_queue_id: (dwords[2] >> 16) as u16,
            command_id: dwords[3] as u16,
            phase: Self::is_phase_tag_set(dwords[3]),
            status: (dwords[3] >> 17) as u16,
        }
    }

    /// True when the phase tag (`P`, bit 16) of a completion entry's dword 3 is set, read before
    /// the rest of the entry. A driver polls dword 3 alone, compares this with
    /// [`crate::CompletionQueue::expects_phase_tag_set`], and reads dwords 0 to 2 only after they
    /// match and a read barrier, so a fresh phase never pairs with a stale head or command
    /// identifier.
    #[doc(alias = "P")]
    #[doc(alias = "phase tag")]
    pub fn is_phase_tag_set(dword_three: u32) -> bool {
        dword_three & (1 << 16) != 0
    }

    /// True when the whole status field is 0: Successful Completion of the generic code type,
    /// with no retry advice and nothing more to read.
    pub fn is_success(&self) -> bool {
        self.status == 0
    }

    /// The status code (`SC`, status field bits 0 to 7, dword 3 bits 17 to 24), read within the
    /// code type [`Completion::status_code_type`] names; for the generic type, for example
    /// [`GENERIC_STATUS_INVALID_FIELD_IN_COMMAND`].
    #[doc(alias = "SC")]
    pub fn status_code(&self) -> u8 {
        self.status as u8
    }

    /// The status code type (`SCT`, status field bits 8 to 10, dword 3 bits 25 to 27), from 0 to
    /// 7; 0 is [`STATUS_CODE_TYPE_GENERIC`].
    #[doc(alias = "SCT")]
    pub fn status_code_type(&self) -> u8 {
        (self.status >> 8) as u8 & 0x7
    }

    /// The Command Retry Delay selector (`CRD`, status field bits 11 to 12, dword 3 bits 28 to
    /// 29), from 0 to 3: 0 asks for no delay before a retry, and 1 to 3 name one of the three
    /// delay times the controller reports in its Identify Controller structure.
    #[doc(alias = "CRD")]
    pub fn command_retry_delay(&self) -> u8 {
        (self.status >> 11) as u8 & 0x3
    }

    /// True when More (`M`, status field bit 13, dword 3 bit 30) is set: the controller holds more
    /// status information for this command in its Error Information log page.
    #[doc(alias = "M")]
    #[doc(alias = "More")]
    pub fn is_more_set(&self) -> bool {
        self.status & (1 << 13) != 0
    }

    /// True when Do Not Retry (`DNR`, status field bit 14, dword 3 bit 31) is set: the controller
    /// says the same command would fail again, so the driver reports the failure instead of
    /// resubmitting.
    #[doc(alias = "DNR")]
    #[doc(alias = "Do Not Retry")]
    pub fn is_do_not_retry_set(&self) -> bool {
        self.status & (1 << 14) != 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::queues::CompletionQueue;

    #[test]
    fn completions_decode_and_the_phase_gates_ownership() {
        let queue = CompletionQueue::new(4);
        // The controller writes phase=1 on its first lap: owned.
        let fresh = Completion::from_dwords([0, 0, 5 | 1 << 16, 42 | 1 << 16]);
        assert_eq!(
            (
                fresh.submission_queue_head,
                fresh.submission_queue_id,
                fresh.command_id
            ),
            (5, 1, 42)
        );
        assert_eq!(fresh.status, 0);
        assert!(queue.is_owned(&fresh));
        // A zeroed (never-written) entry is not.
        let stale = Completion::from_dwords([0; 4]);
        assert!(!queue.is_owned(&stale));
        // An error status decodes: dword 3 bits 17.. carry it.
        let failed = Completion::from_dwords([0, 0, 0, 1 << 16 | 0x2 << 17]);
        assert_eq!(failed.status, 0x2, "Invalid Field in Command");
        assert!(!failed.is_success());
        assert_eq!(
            failed.status_code(),
            GENERIC_STATUS_INVALID_FIELD_IN_COMMAND
        );
        assert_eq!(failed.status_code_type(), STATUS_CODE_TYPE_GENERIC);
    }

    /// Each part of the status field reads its own bits of dword 3 and only those: one field set
    /// to all ones at a time, with every other bit of the status clear.
    #[test]
    fn each_status_part_reads_its_own_bits_of_dword_three() {
        let parts = |dword_three: u32| {
            let completion = Completion::from_dwords([0, 0, 0, dword_three]);
            (
                completion.is_success(),
                completion.status_code(),
                completion.status_code_type(),
                completion.command_retry_delay(),
                completion.is_more_set(),
                completion.is_do_not_retry_set(),
            )
        };
        assert_eq!(parts(0), (true, 0, 0, 0, false, false));
        assert_eq!(parts(1 << 16 | 0xffff), (true, 0, 0, 0, false, false));
        assert_eq!(parts(0xff << 17), (false, 0xff, 0, 0, false, false));
        assert_eq!(parts(0x7 << 25), (false, 0, 7, 0, false, false));
        assert_eq!(parts(0x3 << 28), (false, 0, 0, 3, false, false));
        assert_eq!(parts(1 << 30), (false, 0, 0, 0, true, false));
        assert_eq!(parts(1 << 31), (false, 0, 0, 0, false, true));
        assert_eq!(parts(u32::MAX), (false, 0xff, 7, 3, true, true));
        // A status code type of 1 (command specific) with code 0 is a failure, not a success.
        assert_eq!(parts(1 << 25), (false, 0, 1, 0, false, false));
    }

    #[test]
    fn the_phase_of_dword_three_is_bit_16_alone() {
        assert!(Completion::is_phase_tag_set(1 << 16));
        assert!(!Completion::is_phase_tag_set(!(1 << 16)));
        assert!(Completion::is_phase_tag_set(u32::MAX));
        assert!(!Completion::is_phase_tag_set(0));
    }
}
