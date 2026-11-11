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
    /// The 15-bit status field (`SF`), dword 3 bits 17 to 31; 0 is success. Status code type in bits 8
    /// to 10, code in bits 0 to 7.
    #[doc(alias = "SF")]
    pub status: u16,
}

impl Completion {
    /// Decode the four dwords the controller wrote into a completion queue slot.
    pub fn from_dwords(dwords: [u32; 4]) -> Self {
        Completion {
            result: dwords[0],
            submission_queue_head: dwords[2] as u16,
            submission_queue_id: (dwords[2] >> 16) as u16,
            command_id: dwords[3] as u16,
            phase: dwords[3] & (1 << 16) != 0,
            status: (dwords[3] >> 17) as u16,
        }
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
    }
}
