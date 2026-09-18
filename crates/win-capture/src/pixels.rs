//! Convert the mapped BGRA surface directly into an upright, tightly packed image.

#[derive(Clone, Copy)]
pub(crate) enum Rotation {
    Identity,
    Clockwise90,
    Clockwise180,
    Clockwise270,
}

const INVALID_GEOMETRY: &str = "DXGI returned invalid image geometry";

/// Consume exactly `height` rows of `width * 4` initialized BGRA pixel bytes.
/// Row borrows are used only during this call; the returned image owns its pixels.
pub(crate) fn decode_bgra<'a>(
    rows: impl Iterator<Item = &'a [u8]>,
    width: u32,
    height: u32,
    rotation: Rotation,
) -> Result<image::RgbaImage, &'static str> {
    if width == 0 || height == 0 {
        return Err(INVALID_GEOMETRY);
    }
    let w = usize::try_from(width).map_err(|_| INVALID_GEOMETRY)?;
    let h = usize::try_from(height).map_err(|_| INVALID_GEOMETRY)?;
    let row_bytes = w.checked_mul(4).ok_or(INVALID_GEOMETRY)?;
    let byte_len = row_bytes
        .checked_mul(h)
        .filter(|length| *length <= isize::MAX as usize)
        .ok_or(INVALID_GEOMETRY)?;
    // Consume source rows once and scatter directly into the sole RGBA allocation.
    // Choose the mapping outside the pixel loop; each closure is monomorphized.
    let mut pixels = vec![0; byte_len];
    let dimensions = match rotation {
        Rotation::Identity => {
            convert(rows, row_bytes, h, &mut pixels, |x, y| y * w + x)?;
            (width, height)
        }
        Rotation::Clockwise90 => {
            convert(rows, row_bytes, h, &mut pixels, |x, y| x * h + (h - 1 - y))?;
            (height, width)
        }
        Rotation::Clockwise180 => {
            convert(rows, row_bytes, h, &mut pixels, |x, y| {
                (h - 1 - y) * w + (w - 1 - x)
            })?;
            (width, height)
        }
        Rotation::Clockwise270 => {
            convert(rows, row_bytes, h, &mut pixels, |x, y| (w - 1 - x) * h + y)?;
            (height, width)
        }
    };
    image::RgbaImage::from_raw(dimensions.0, dimensions.1, pixels).ok_or(INVALID_GEOMETRY)
}

fn convert<'a>(
    mut rows: impl Iterator<Item = &'a [u8]>,
    row_bytes: usize,
    height: usize,
    pixels: &mut [u8],
    destination_at: impl Fn(usize, usize) -> usize,
) -> Result<(), &'static str> {
    for y in 0..height {
        let row = rows
            .next()
            .filter(|row| row.len() == row_bytes)
            .ok_or(INVALID_GEOMETRY)?;
        for (x, pixel) in row.chunks_exact(4).enumerate() {
            let at = destination_at(x, y) * 4;
            pixels[at..at + 4].copy_from_slice(&[pixel[2], pixel[1], pixel[0], pixel[3]]);
        }
    }
    if rows.next().is_some() {
        return Err(INVALID_GEOMETRY);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROWS: [&[u8]; 2] = [
        &[30, 20, 10, 255, 60, 50, 40, 254, 90, 80, 70, 253],
        &[120, 110, 100, 252, 150, 140, 130, 251, 180, 170, 160, 250],
    ];

    #[test]
    fn bgra_rows_become_rgba_preserving_alpha() {
        let image = decode_bgra(ROWS.into_iter(), 3, 2, Rotation::Identity).unwrap();
        assert_eq!(image.dimensions(), (3, 2));
        assert_eq!(
            image.into_raw(),
            [
                10, 20, 30, 255, 40, 50, 60, 254, 70, 80, 90, 253, 100, 110, 120, 252, 130, 140,
                150, 251, 160, 170, 180, 250,
            ]
        );
    }

    #[test]
    fn rotated_monitors_return_upright_pixel_positions() {
        // Red channels label the source grid: A B C / D E F. Expected
        // coordinates are hand-derived, not computed by a rotation helper.
        for (rotation, dimensions, red) in [
            (
                Rotation::Clockwise90,
                (2, 3),
                vec![100, 10, 130, 40, 160, 70],
            ),
            (
                Rotation::Clockwise180,
                (3, 2),
                vec![160, 130, 100, 70, 40, 10],
            ),
            (
                Rotation::Clockwise270,
                (2, 3),
                vec![70, 160, 40, 130, 10, 100],
            ),
        ] {
            let image = decode_bgra(ROWS.into_iter(), 3, 2, rotation).unwrap();
            assert_eq!(image.dimensions(), dimensions);
            assert_eq!(
                image.pixels().map(|pixel| pixel[0]).collect::<Vec<_>>(),
                red
            );
        }
    }

    #[test]
    fn malformed_rows_are_rejected() {
        for rows in [
            [&ROWS[0][..11], ROWS[1]],
            [ROWS[0], &ROWS[1][..11]],
            [ROWS[0], &[0_u8; 13]],
        ] {
            assert!(decode_bgra(rows.into_iter(), 3, 2, Rotation::Identity).is_err());
        }
        assert!(decode_bgra(ROWS[..1].iter().copied(), 3, 2, Rotation::Identity).is_err());
        assert!(decode_bgra(ROWS.into_iter(), 3, 1, Rotation::Identity).is_err());
    }

    #[test]
    fn empty_or_overflowing_geometry_is_rejected_before_consuming_rows() {
        for (width, height) in [(0, 2), (3, 0), (1 << 31, 1 << 30), (u32::MAX, u32::MAX)] {
            let rows = std::iter::from_fn(|| -> Option<&[u8]> {
                panic!("invalid image geometry must not consume rows")
            });
            assert!(decode_bgra(rows, width, height, Rotation::Identity).is_err());
        }
    }
}
