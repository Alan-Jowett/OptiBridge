use core::ops::Range;

use optibridge_protocol::BpfMapMetadata;

pub const HELPER_ERROR: u64 = u64::MAX;

pub fn value_range(map: &BpfMapMetadata, entry: u32) -> Option<Range<usize>> {
    if map.key_size != 4 {
        return None;
    }
    let value_size = map.value_size as usize;
    let offset = (entry as usize).checked_mul(value_size)?;
    let end = offset.checked_add(value_size)?;
    if end > map.backing_len as usize {
        return None;
    }
    Some(offset..end)
}

pub fn lookup<'a>(map: &BpfMapMetadata, entry: u32, backing: &'a [u8]) -> Option<&'a [u8]> {
    let range = value_range(map, entry)?;
    let start = map.backing_offset as usize;
    backing.get(start + range.start..start + range.end)
}

pub fn update(
    map: &BpfMapMetadata,
    entry: u32,
    value: &[u8],
    backing: &mut [u8],
) -> Result<(), ()> {
    if value.len() != map.value_size as usize {
        return Err(());
    }
    let range = value_range(map, entry).ok_or(())?;
    let start = map.backing_offset as usize;
    backing
        .get_mut(start + range.start..start + range.end)
        .ok_or(())?
        .copy_from_slice(value);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{lookup, update, value_range};
    use optibridge_protocol::BpfMapMetadata;

    fn map() -> BpfMapMetadata {
        BpfMapMetadata {
            backing_offset: 4,
            backing_len: 8,
            key_size: 4,
            value_size: 4,
        }
    }

    #[test]
    fn array_entries_are_fixed_size_and_bounded() {
        assert_eq!(value_range(&map(), 0), Some(0..4));
        assert_eq!(value_range(&map(), 1), Some(4..8));
        assert_eq!(value_range(&map(), 2), None);
    }

    #[test]
    fn lookup_and_update_use_value_ranges() {
        let metadata = map();
        let mut backing = [0; 12];
        assert_eq!(lookup(&metadata, 0, &backing), Some(&[0, 0, 0, 0][..]));
        assert_eq!(update(&metadata, 1, &[1, 2, 3, 4], &mut backing), Ok(()));
        assert_eq!(lookup(&metadata, 1, &backing), Some(&[1, 2, 3, 4][..]));
        assert_eq!(&backing[..4], &[0, 0, 0, 0]);
        assert_eq!(update(&metadata, 0, &[1, 2], &mut backing), Err(()));
    }
}
