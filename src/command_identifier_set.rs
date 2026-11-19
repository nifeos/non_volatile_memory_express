//! **The command identifier set**: which identifiers the driver has handed to commands the
//! controller has not yet completed.

/// A fixed set of command identifiers, `64 * WORDS` of them (0 to `64 * WORDS - 1`), each either
/// free or in flight, kept as a bitmap of `WORDS` words. The driver acquires an identifier with
/// [`CommandIdentifierSet::acquire`] when it pushes a command and releases it with [`CommandIdentifierSet::release`] when an
/// owned completion carries it back, so an identifier is never on two commands at once.
///
/// The completion's identifier comes from the controller, so it is checked rather than trusted:
/// releasing one that is not in flight (never acquired, already released, or beyond the set) is a
/// [`NotInFlightError`] and changes nothing. One set serves one submission queue, since a
/// completion names its command by the pair of submission queue and command identifier.
///
/// `WORDS` lies from 1 to 1024, so that every identifier fits the 16-bit field; the compiler
/// refuses any other.
///
/// # Examples
///
/// ```
/// use non_volatile_memory_express::CommandIdentifierSet;
///
/// let mut identifiers = CommandIdentifierSet::<1>::new(); // identifiers 0 to 63
/// let first = identifiers.acquire().expect("the set is empty");
/// let second = identifiers.acquire().expect("the set has room");
/// assert_eq!((first, second, identifiers.len()), (0, 1, 2));
///
/// // The completion for `first` arrives: its identifier is released, once.
/// assert!(identifiers.release(first).is_ok());
/// assert!(identifiers.release(first).is_err()); // a second completion for it is refused
/// assert!(identifiers.release(40).is_err()); // never acquired
/// assert!(identifiers.contains(second) && !identifiers.contains(first));
/// assert_eq!(identifiers.acquire(), Some(0)); // the lowest free identifier comes back first
/// ```
///
/// A set of no words, or of more than 1024, does not compile:
///
/// ```compile_fail
/// let _ = non_volatile_memory_express::CommandIdentifierSet::<1025>::new();
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[doc(alias = "CID")]
pub struct CommandIdentifierSet<const WORDS: usize> {
    pub(crate) words: [u64; WORDS],
}

impl<const WORDS: usize> CommandIdentifierSet<WORDS> {
    /// The number of identifiers the set holds, in flight or free.
    pub const CAPACITY: usize = 64 * WORDS;

    /// An empty set: every identifier free.
    pub const fn new() -> Self {
        const {
            assert!(
                WORDS >= 1 && WORDS <= 1024,
                "a command identifier set holds 64 to 65536 command identifiers"
            );
        }
        CommandIdentifierSet { words: [0; WORDS] }
    }

    /// Acquire the lowest free identifier and mark it in flight. `None` when every identifier is in
    /// flight, and the set is unchanged: the driver waits for a completion before it pushes more.
    pub fn acquire(&mut self) -> Option<u16> {
        for (index, word) in self.words.iter_mut().enumerate() {
            if *word != u64::MAX {
                let bit = word.trailing_ones();
                *word |= 1 << bit;
                return Some((index * 64) as u16 + bit as u16);
            }
        }
        None
    }

    /// Release `command_id`, from an owned completion, so it can be acquired again. A
    /// [`NotInFlightError`] if it is not in flight, with the set unchanged.
    pub fn release(&mut self, command_id: u16) -> Result<(), NotInFlightError> {
        if !self.contains(command_id) {
            return Err(NotInFlightError { command_id });
        }
        self.words[usize::from(command_id) / 64] &= !(1 << (command_id % 64));
        Ok(())
    }

    /// Is `command_id` in flight? False for one beyond the set.
    pub fn contains(&self, command_id: u16) -> bool {
        self.words
            .get(usize::from(command_id) / 64)
            .is_some_and(|word| word & (1 << (command_id % 64)) != 0)
    }

    /// How many identifiers are in flight.
    pub fn len(&self) -> usize {
        self.words
            .iter()
            .map(|word| word.count_ones() as usize)
            .sum()
    }

    /// True when no identifier is in flight.
    pub fn is_empty(&self) -> bool {
        self.words.iter().all(|word| *word == 0)
    }
}

/// The empty set, as [`CommandIdentifierSet::new`] builds it.
impl<const WORDS: usize> Default for CommandIdentifierSet<WORDS> {
    fn default() -> Self {
        Self::new()
    }
}

/// **A command identifier that is not in flight**, refused by [`CommandIdentifierSet::release`]: never
/// acquired, already released, or beyond the set. A completion that carries one answers no command
/// the driver is waiting on, so the controller is broken or hostile, and the driver decides what
/// to do with it; the set itself is unchanged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct NotInFlightError {
    /// The identifier the completion carried.
    pub command_id: u16,
}

impl core::fmt::Display for NotInFlightError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            formatter,
            "a completion carried command identifier {}, which is not in flight",
            self.command_id
        )
    }
}

impl core::error::Error for NotInFlightError {}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;
    use std::string::ToString;

    #[test]
    fn acquire_hands_out_every_identifier_once_lowest_first_then_refuses() {
        let mut identifiers = CommandIdentifierSet::<2>::new();
        assert!(identifiers.is_empty());
        assert_eq!(identifiers, CommandIdentifierSet::default());
        assert_eq!(CommandIdentifierSet::<2>::CAPACITY, 128);
        for expected in 0..128u16 {
            assert_eq!(identifiers.acquire(), Some(expected));
            assert!(identifiers.contains(expected));
            assert_eq!(identifiers.len(), usize::from(expected) + 1);
            assert!(!identifiers.is_empty());
        }
        let full = identifiers;
        assert_eq!(identifiers.acquire(), None);
        assert_eq!(identifiers, full, "a refused acquire changes nothing");
    }

    #[test]
    fn release_frees_exactly_one_identifier_and_refuses_one_not_in_flight() {
        let mut identifiers = CommandIdentifierSet::<2>::new();
        for _ in 0..128 {
            identifiers.acquire();
        }
        assert_eq!(identifiers.release(70), Ok(()));
        assert!(!identifiers.contains(70) && identifiers.contains(69) && identifiers.contains(71));
        assert_eq!(identifiers.len(), 127);
        let before = identifiers;
        assert_eq!(
            identifiers.release(70),
            Err(NotInFlightError { command_id: 70 })
        );
        assert_eq!(
            identifiers.release(128),
            Err(NotInFlightError { command_id: 128 })
        );
        assert_eq!(identifiers, before, "a refused release changes nothing");
        assert!(!identifiers.contains(u16::MAX));
        assert_eq!(
            identifiers.acquire(),
            Some(70),
            "the freed identifier is the lowest free"
        );
    }

    #[test]
    fn the_largest_set_names_every_16_bit_identifier() {
        let mut identifiers = CommandIdentifierSet::<1024>::new();
        assert_eq!(CommandIdentifierSet::<1024>::CAPACITY, 65536);
        identifiers.words = [u64::MAX; 1024];
        identifiers.words[1023] = !(1 << 63);
        assert_eq!(identifiers.acquire(), Some(u16::MAX));
        assert_eq!(identifiers.acquire(), None);
        assert_eq!(identifiers.release(u16::MAX), Ok(()));
    }

    #[test]
    fn the_refusal_names_the_identifier() {
        let error = NotInFlightError { command_id: 9 };
        assert_eq!(
            error.to_string(),
            "a completion carried command identifier 9, which is not in flight"
        );
    }
}
