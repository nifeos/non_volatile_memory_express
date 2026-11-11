//! **Physical Region Page pointers** (section 4.3, Physical Region Page Entry and List): the two
//! pointer words a data transfer carries, and the list a longer transfer needs.

/// **The two Physical Region Page pointers of one transfer**: PRP1 and PRP2, as a command carries
/// them. `second_entry` is zero when one page holds the whole transfer.
#[doc(alias = "PRP")]
#[doc(alias = "physical region page")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PhysicalRegionPagePair {
    /// PRP1: the transfer's first byte, which may carry an offset into its page.
    pub first_entry: u64,
    /// PRP2: the next page boundary when the transfer spills into a second page, otherwise zero.
    pub second_entry: u64,
}

/// **Physical Region Page pair** (PRP1 and PRP2, section 4.3, Physical Region Page Entry and
/// List): the two PRP words for one data transfer of `length_bytes` at physical
/// `base_physical_address`, against pages of `page_size_bytes`. PRP1 may carry an offset; every
/// later page pointer must not. One page of data needs only PRP1; a transfer spilling into a
/// second page puts that page in PRP2; anything longer needs a PRP *list*, which
/// [`physical_region_page_list`] builds, so `None` here says "not expressible as a pair".
///
/// # Panics
///
/// On a `page_size_bytes` that is not a power of two of at least 512 bytes. The page size is the
/// caller's configuration, never a value the controller writes.
///
/// # Examples
///
/// ```
/// use non_volatile_memory_express::{PhysicalRegionPagePair, physical_region_page_pair};
///
/// // A page-aligned block: one pointer is enough.
/// let aligned = physical_region_page_pair(0x4000_0000, 4096, 4096).unwrap();
/// assert_eq!(aligned, PhysicalRegionPagePair { first_entry: 0x4000_0000, second_entry: 0 });
///
/// // The same block, offset half a page: it spills, so the second entry carries the next page
/// // boundary, with no offset of its own. That "offset-free by construction" is the
/// // specification's rule for PRP2.
/// let straddling = physical_region_page_pair(0x4000_0800, 4096, 4096).unwrap();
/// assert_eq!(straddling.first_entry, 0x4000_0800);
/// assert_eq!(straddling.second_entry, 0x4000_1000);
///
/// // Three pages would need a list. Not expressible, and said so.
/// assert!(physical_region_page_pair(0x4000_0000, 3 * 4096, 4096).is_none());
/// ```
#[doc(alias = "physical region page")]
#[doc(alias = "PRP")]
pub fn physical_region_page_pair(
    base_physical_address: u64,
    length_bytes: u64,
    page_size_bytes: u64,
) -> Option<PhysicalRegionPagePair> {
    assert!(
        page_size_bytes.is_power_of_two() && page_size_bytes >= 512,
        "a page is a power of two of at least 512 bytes"
    );
    if length_bytes == 0 {
        return None;
    }
    let offset = base_physical_address & (page_size_bytes - 1);
    let end = offset.checked_add(length_bytes)?;
    if end <= page_size_bytes {
        Some(PhysicalRegionPagePair {
            first_entry: base_physical_address,
            second_entry: 0,
        })
    } else if end <= 2 * page_size_bytes {
        // The second pointer is the next page boundary after the base, offset-free by
        // construction. Checked: a base in the last page of the address space has no next
        // boundary.
        Some(PhysicalRegionPagePair {
            first_entry: base_physical_address,
            second_entry: (base_physical_address & !(page_size_bytes - 1))
                .checked_add(page_size_bytes)?,
        })
    } else {
        None // would need a PRP list
    }
}

/// **Physical Region Page list**: the PRP1 and PRP2 of one transfer of `length_bytes` at physical
/// `base_physical_address`, against pages of `page_size_bytes`, for a transfer of any length one
/// list page can carry. A transfer of one or two pages gets the same pair as
/// [`physical_region_page_pair`], and `list` is left as it was. A longer one gets PRP1 as the base
/// and PRP2 as `list_physical_address`, and the function writes the transfer's later pages, one
/// page-aligned entry each in order, into the start of `list`, the memory the caller has mapped at
/// `list_physical_address`. The caller keeps that memory unchanged until the command completes.
///
/// `None`, with `list` unchanged, for an empty transfer, a list address that is not 8-byte
/// aligned, a list longer than `list` holds, a list that would cross from its page into the
/// next (which the specification chains through a pointer in the page's last entry, and this
/// function does not build), or a transfer whose last page lies beyond the address space. A list
/// page-aligned in memory holds `page_size_bytes / 8` entries, so a 4 KiB page carries a transfer
/// of up to 513 pages, about 2 MiB.
///
/// # Panics
///
/// On a `page_size_bytes` that is not a power of two of at least 512 bytes, as
/// [`physical_region_page_pair`] does.
///
/// # Examples
///
/// ```
/// use non_volatile_memory_express::{PhysicalRegionPagePair, physical_region_page_list};
///
/// // Three pages starting half a page in: PRP1 carries the offset, and the list the two pages
/// // after it.
/// let mut list = [0u64; 512]; // one 4 KiB page, mapped at physical 0x9000
/// let pair = physical_region_page_list(0x4000_0800, 3 * 4096, 4096, &mut list, 0x9000).unwrap();
/// assert_eq!(pair, PhysicalRegionPagePair { first_entry: 0x4000_0800, second_entry: 0x9000 });
/// assert_eq!(list[..4], [0x4000_1000, 0x4000_2000, 0x4000_3000, 0]);
/// ```
#[doc(alias = "physical region page")]
#[doc(alias = "PRP")]
#[doc(alias = "PRP list")]
pub fn physical_region_page_list(
    base_physical_address: u64,
    length_bytes: u64,
    page_size_bytes: u64,
    list: &mut [u64],
    list_physical_address: u64,
) -> Option<PhysicalRegionPagePair> {
    assert!(
        page_size_bytes.is_power_of_two() && page_size_bytes >= 512,
        "a page is a power of two of at least 512 bytes"
    );
    if length_bytes == 0 {
        return None;
    }
    let offset = base_physical_address & (page_size_bytes - 1);
    let pages = offset.checked_add(length_bytes)?.div_ceil(page_size_bytes);
    if pages <= 2 {
        return physical_region_page_pair(base_physical_address, length_bytes, page_size_bytes);
    }
    // Every page after the first takes one entry, and the entries must stay in the list's page.
    let entries = pages - 1;
    let room = (page_size_bytes - (list_physical_address & (page_size_bytes - 1))) / 8;
    if list_physical_address % 8 != 0 || entries > room || entries > list.len() as u64 {
        return None;
    }
    let first_page = base_physical_address & !(page_size_bytes - 1);
    first_page.checked_add(entries.checked_mul(page_size_bytes)?)?;
    let mut page = first_page;
    for entry in &mut list[..entries as usize] {
        page += page_size_bytes;
        *entry = page;
    }
    Some(PhysicalRegionPagePair {
        first_entry: base_physical_address,
        second_entry: list_physical_address,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pair(first_entry: u64, second_entry: u64) -> Option<PhysicalRegionPagePair> {
        Some(PhysicalRegionPagePair {
            first_entry,
            second_entry,
        })
    }

    #[test]
    fn pairs_cover_one_and_two_pages_and_refuse_more() {
        // A page-aligned single page: PRP1 only.
        assert_eq!(
            physical_region_page_pair(0x5000, 4096, 4096),
            pair(0x5000, 0)
        );
        // An offset transfer spilling into the next page: PRP2 is that page's boundary.
        assert_eq!(
            physical_region_page_pair(0x5800, 4096, 4096),
            pair(0x5800, 0x6000)
        );
        // Exactly two aligned pages: still expressible.
        assert_eq!(
            physical_region_page_pair(0x5000, 8192, 4096),
            pair(0x5000, 0x6000)
        );
        // More needs a PRP list, which this crate does not build.
        assert_eq!(physical_region_page_pair(0x5800, 8192, 4096), None);
        assert_eq!(
            physical_region_page_pair(0x5000, 0, 4096),
            None,
            "empty transfers refused"
        );
    }

    #[test]
    fn a_list_names_every_page_after_the_first_and_nothing_more() {
        let mut list = [0u64; 8];
        // Three aligned pages: the two after the first, in order.
        assert_eq!(
            physical_region_page_list(0x10000, 3 * 4096, 4096, &mut list, 0x2000),
            pair(0x10000, 0x2000)
        );
        assert_eq!(list, [0x11000, 0x12000, 0, 0, 0, 0, 0, 0]);
        // Three pages from an offset base: four pages spanned, three entries.
        let mut list = [0u64; 8];
        assert_eq!(
            physical_region_page_list(0x10800, 3 * 4096, 4096, &mut list, 0x2008),
            pair(0x10800, 0x2008)
        );
        assert_eq!(list[..4], [0x11000, 0x12000, 0x13000, 0]);
        // A page size other than 4 KiB walks its own pages.
        let mut list = [0u64; 8];
        assert!(physical_region_page_list(0x40000, 3 * 16384, 16384, &mut list, 0).is_some());
        assert_eq!(list[..3], [0x44000, 0x48000, 0]);
    }

    #[test]
    fn one_or_two_pages_are_the_pair_and_leave_the_list_alone() {
        let mut list = [7u64; 4];
        for (base, length) in [(0x5000, 4096), (0x5800, 4096), (0x5000, 8192), (0x5fff, 2)] {
            assert_eq!(
                physical_region_page_list(base, length, 4096, &mut list, 0x2000),
                physical_region_page_pair(base, length, 4096)
            );
        }
        assert_eq!(list, [7; 4]);
    }

    #[test]
    fn a_list_that_cannot_be_built_is_refused_and_writes_nothing() {
        let mut list = [7u64; 4];
        let refused = |base, length, list_physical_address, list: &mut [u64]| {
            assert_eq!(
                physical_region_page_list(base, length, 4096, list, list_physical_address),
                None
            );
        };
        refused(0x10000, 0, 0x2000, &mut list); // empty
        refused(0x10000, 3 * 4096, 0x2004, &mut list); // list address not 8-byte aligned
        refused(0x10000, 6 * 4096, 0x2000, &mut list); // five entries, four slots
        refused(0x10000, 4 * 4096, 0x2ff0, &mut list); // three entries, two left in the page
        refused(!0xfff, 3 * 4096, 0x2000, &mut list); // pages past the address space
        refused(0x800, u64::MAX, 0x2000, &mut list); // the end itself wraps
        assert_eq!(list, [7; 4]);
        // The entries that fit exactly at the end of the list's page are accepted.
        assert!(physical_region_page_list(0x10000, 3 * 4096, 4096, &mut list, 0x2ff0).is_some());
        assert_eq!(list, [0x11000, 0x12000, 7, 7]);
        // So are entries that exactly fill the slice the caller gave.
        let mut exact = [0u64; 2];
        assert!(physical_region_page_list(0x10000, 3 * 4096, 4096, &mut exact, 0x2000).is_some());
        assert_eq!(exact, [0x11000, 0x12000]);
    }

    #[test]
    fn a_page_aligned_list_carries_one_entry_per_eight_bytes_of_its_page() {
        let mut list = [0u64; 513];
        assert!(physical_region_page_list(0, 513 * 4096, 4096, &mut list, 0x1000).is_some());
        assert_eq!(list[511], 512 * 4096);
        assert_eq!(
            list[512], 0,
            "the 513th slot is outside the list's page and stays unwritten"
        );
        assert_eq!(
            physical_region_page_list(0, 514 * 4096, 4096, &mut list, 0x1000),
            None
        );
    }

    #[test]
    #[should_panic(expected = "a page is a power of two of at least 512 bytes")]
    fn a_list_against_a_page_that_is_not_a_power_of_two_panics() {
        physical_region_page_list(0, 4096, 3000, &mut [0; 4], 0);
    }
}
