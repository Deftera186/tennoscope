//! Borrow only initialized BGRA pixel bytes from each row of a live mapping.

/// Return one exact pixel slice per row, tied to the mapping owner's borrow.
/// Neither inter-row padding nor trailing padding is ever borrowed as bytes.
///
/// # Safety
///
/// If the pointer and layout pass validation, `data` must originate from one
/// allocation covering `(height - 1) * pitch + width * 4` bytes. Each row's first
/// `width * 4` bytes must be initialized. The allocation must remain live and those
/// pixel bytes immutable for the entire borrow of `owner`, including all returned
/// row slices. Padding may be uninitialized, and final-row padding may be absent.
/// The caller must ensure `owner` actually controls that memory's lifetime.
pub(crate) unsafe fn bgra_rows<'a, T: ?Sized>(
    _owner: &'a T,
    data: *const u8,
    width: u32,
    height: u32,
    pitch: usize,
) -> Result<impl Iterator<Item = &'a [u8]> + 'a, &'static str> {
    let invalid = "Invalid mapped desktop buffer";
    let width = usize::try_from(width).map_err(|_| invalid)?;
    let height = usize::try_from(height).map_err(|_| invalid)?;
    let row_bytes = width.checked_mul(4).ok_or(invalid)?;
    if width == 0 || height == 0 || pitch < row_bytes || data.is_null() {
        return Err(invalid);
    }
    let length = (height - 1)
        .checked_mul(pitch)
        .and_then(|offset| offset.checked_add(row_bytes))
        .filter(|length| *length <= isize::MAX as usize)
        .ok_or(invalid)?;
    if data.addr().checked_add(length).is_none() {
        return Err(invalid);
    }
    Ok((0..height).map(move |row| {
        // SAFETY: validated arithmetic keeps this row inside the single mapped
        // allocation supplied by the caller. u8 has alignment 1; the pointer is
        // non-null, the slice is <= isize::MAX, and its address cannot wrap.
        // Only initialized pixels are included, never the intervening padding.
        // The caller keeps pixels live and immutable for the owner's borrow.
        unsafe { std::slice::from_raw_parts(data.add(row * pitch), row_bytes) }
    }))
}

#[cfg(test)]
mod tests {
    use std::mem::MaybeUninit;

    use super::*;
    use crate::pixels::{Rotation, decode_bgra};

    #[test]
    fn uninitialized_padding_is_never_borrowed_as_pixel_bytes() {
        // The final row has no trailing padding. Inter-row bytes 12..16 stay
        // genuinely uninitialized, so Miri can catch a full-span &[u8] borrow.
        let mut storage = [MaybeUninit::<u8>::uninit(); 28];
        for (offset, pixels) in [
            (0, [30, 20, 10, 255, 60, 50, 40, 255, 90, 80, 70, 255]),
            (
                16,
                [120, 110, 100, 255, 150, 140, 130, 255, 180, 170, 160, 255],
            ),
        ] {
            for (slot, byte) in storage[offset..offset + pixels.len()]
                .iter_mut()
                .zip(pixels)
            {
                slot.write(byte);
            }
        }

        for (rotation, dimensions, red) in [
            (Rotation::Identity, (3, 2), [10, 40, 70, 100, 130, 160]),
            (Rotation::Clockwise90, (2, 3), [100, 10, 130, 40, 160, 70]),
            (Rotation::Clockwise180, (3, 2), [160, 130, 100, 70, 40, 10]),
            (Rotation::Clockwise270, (2, 3), [70, 160, 40, 130, 10, 100]),
        ] {
            // SAFETY: both 12-byte pixel rows are initialized within storage;
            // only the inter-row padding is uninitialized. The shared storage
            // borrow keeps all pixels alive and immutable throughout decoding.
            let rows = unsafe { bgra_rows(&storage, storage.as_ptr().cast(), 3, 2, 16) }.unwrap();
            let image = decode_bgra(rows, 3, 2, rotation).unwrap();
            assert_eq!(image.dimensions(), dimensions);
            assert_eq!(
                image.pixels().copied().collect::<Vec<_>>(),
                red.map(|red| image::Rgba([red, red + 10, red + 20, 255]))
            );
        }
    }

    #[test]
    fn tightly_packed_rows_accept_pitch_equal_to_pixel_width() {
        let storage = [3_u8, 2, 1, 4, 7, 6, 5, 8];
        // SAFETY: storage contains two initialized 4-byte rows without padding
        // and remains immutably borrowed through conversion.
        let rows = unsafe { bgra_rows(&storage, storage.as_ptr(), 1, 2, 4) }.unwrap();
        let image = decode_bgra(rows, 1, 2, Rotation::Identity).unwrap();
        assert_eq!(image.into_raw(), [1, 2, 3, 4, 5, 6, 7, 8]);
    }

    #[test]
    fn single_row_does_not_require_trailing_pitch_bytes() {
        let storage = [3_u8, 2, 1, 4];
        // SAFETY: only the initialized first row is needed, so the unused stride
        // need not fit inside storage. Its pixels stay immutable during decoding.
        let rows = unsafe { bgra_rows(&storage, storage.as_ptr(), 1, 1, usize::MAX) }.unwrap();
        let image = decode_bgra(rows, 1, 1, Rotation::Identity).unwrap();
        assert_eq!(image.into_raw(), [1, 2, 3, 4]);
    }

    #[test]
    fn invalid_mapped_layouts_are_rejected_without_reading() {
        let storage = [0_u8; 4];
        for (width, height, pitch) in [
            (0, 1, 4),
            (1, 0, 4),
            (2, 1, 7),
            (1, 2, usize::MAX),
            (1, 2, isize::MAX as usize),
            (u32::MAX, u32::MAX, usize::MAX),
        ] {
            // SAFETY: every supplied layout is invalid; no rows can be returned
            // and the pointer is never dereferenced.
            assert!(
                unsafe { bgra_rows(&storage, storage.as_ptr(), width, height, pitch) }.is_err()
            );
        }
        // SAFETY: a null pointer is rejected before any row can be returned.
        assert!(unsafe { bgra_rows(&storage, std::ptr::null(), 1, 1, 4) }.is_err());
        let wrapping = std::ptr::without_provenance::<u8>(usize::MAX - 2);
        // SAFETY: the four-byte row would wrap the address space, so validation
        // rejects this pointer without accessing it or returning any rows.
        assert!(unsafe { bgra_rows(&storage, wrapping, 1, 1, 4) }.is_err());
    }
}
