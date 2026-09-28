//! Read the kiosk's item names off the screen.
//!
//! Same philosophy as the reward-card reader, one important difference: the reward screen is
//! read all-or-nothing because four cards are one squad's choice, while the kiosk is read
//! per-slot. A hover card covers two tiles, a filtered inventory leaves the last row short, and
//! a scroll parks rows half out of view -- none of those may take the other chips down with
//! them, so every crop stands on its own and slots that do not clear the match floor simply
//! render nothing.
//!
//! It is not general OCR: every label only has to land on the nearest of the player's own prime
//! parts (`CatalogIndex::reward_entries`), which is what makes a garbled read safe to drop.

use image::{DynamicImage, GenericImageView, GrayImage};
use std::path::PathBuf;

use warframe_acquisition::RewardCatalogEntry;

use crate::{
    kiosk_geometry::{basket_label_rect, basket_quantity_rect},
    reward_ocr::{best_match, ocr_crop, ocr_crop_block, prepare_crop},
};

/// Below this a slot's read is treated as absent rather than published as a guess; same floor as
/// the reward reader.
pub const MATCH_FLOOR: f32 = 0.6;

/// One recognized grid tile: its slot, the catalog name it matched, and how well.
#[derive(Clone, Debug)]
pub struct GridCell {
    pub col: usize,
    pub row: usize,
    pub name: String,
    pub score: f32,
}

/// One recognized basket row, including Warframe's optional stack count.
#[derive(Clone, Debug)]
pub struct BasketRow {
    pub index: usize,
    pub name: String,
    pub score: f32,
    pub quantity: u32,
}

/// Distinguishes concurrent readers' scratch crops (same reason as `reward_ocr::SCRATCH`).
static SCRATCH: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn scratch_file() -> PathBuf {
    let ticket = SCRATCH.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "tennoscope-kiosk-crop-{}-{ticket}.png",
        std::process::id()
    ))
}

/// Read every grid slot independently; slots below the floor are simply not returned.
///
/// The slots share nothing but the source frame, and one tesseract spawn costs about as much
/// as the whole crop's preprocessing, so the reads run across a small pool of threads: the
/// poller's whole budget is one interval, and 24 sequential spawns spend several of them.
pub fn read_grid(
    image: &DynamicImage,
    candidates: &[RewardCatalogEntry],
    dy: i32,
) -> Vec<GridCell> {
    let luma = image.to_luma8();
    let (width, height) = image.dimensions();
    let slots: Vec<(usize, usize)> = (0..crate::kiosk_geometry::GRID_ROWS)
        .flat_map(|row| (0..crate::kiosk_geometry::GRID_COLS).map(move |col| (col, row)))
        .collect();
    let reads = read_slots(&luma, image, width, height, &slots, candidates, |slot| {
        crate::kiosk_geometry::grid_label_rect(width, height, slot.0, slot.1, dy)
    });
    slots
        .into_iter()
        .zip(reads)
        .filter_map(|((col, row), read)| {
            read.map(|read| GridCell {
                col,
                row,
                name: read.name,
                score: read.score,
            })
        })
        .collect()
}

/// Read every visible basket row independently; rows past the list's end read blank and drop out.
pub fn read_basket(image: &DynamicImage, candidates: &[RewardCatalogEntry]) -> Vec<BasketRow> {
    let luma = image.to_luma8();
    let (width, height) = image.dimensions();
    let slots: Vec<usize> = (0..crate::kiosk_geometry::BASKET_ROWS).collect();
    let reads = read_slots(&luma, image, width, height, &slots, candidates, |index| {
        basket_label_rect(width, height, *index)
    });
    let recognized: Vec<(usize, SlotRead)> = slots
        .into_iter()
        .zip(reads)
        .filter_map(|(index, read)| read.map(|read| (index, read)))
        .collect();
    let quantities = read_quantities(image, &recognized);
    recognized
        .into_iter()
        .zip(quantities)
        .map(|((index, read), quantity)| BasketRow {
            index,
            quantity,
            name: read.name,
            score: read.score,
        })
        .collect()
}

/// Quantity OCR is independent per row. Keep its process launches bounded by the basket's
/// sixteen-row capacity instead of serially adding one Tesseract invocation per row.
fn read_quantities(image: &DynamicImage, rows: &[(usize, SlotRead)]) -> Vec<u32> {
    std::thread::scope(|scope| {
        let handles: Vec<_> = rows
            .iter()
            .map(|(index, _)| scope.spawn(move || read_quantity(image, *index).unwrap_or(1)))
            .collect();
        handles
            .into_iter()
            .map(|handle| handle.join().expect("kiosk quantity ocr worker"))
            .collect()
    })
}

/// Read the optional `N X` stack marker anywhere inside the bounded basket-label band. The
/// band is read in the full charset, like the name lane: letters stay letters, so unlike
/// the old digit-whitelist lane a name initial can never become a count. Only the
/// game's own marker forms digit+X.
fn read_quantity(image: &DynamicImage, index: usize) -> Option<u32> {
    let (x, y, width, height) = basket_quantity_rect(image.width(), image.height(), index)?;
    read_quantity_crop(image, x, y, width, height)
}

fn read_quantity_crop(
    image: &DynamicImage,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
) -> Option<u32> {
    if width == 0 || height == 0 || x + width > image.width() || y + height > image.height() {
        return None;
    }
    // Raw pixels, not the name lane's threshold pipeline: thresholding eats thin stack
    // digits. Full charset, so letters stay letters and only the game's own marker
    // forms digit+X.
    let crop = scratch_file();
    image.crop_imm(x, y, width, height).save(&crop).ok()?;
    let text = ocr_crop(&crop);
    let _ = std::fs::remove_file(&crop);
    let text = text.ok()?;
    Some(basket_quantity(&text)).filter(|&quantity| quantity > 1)
}

struct SlotRead {
    name: String,
    score: f32,
}

/// Read every slot's rect in parallel, preserving input order; failed or sub-floor reads come
/// back as `None` and simply drop out of the caller's view.
fn read_slots<T>(
    luma: &GrayImage,
    image: &DynamicImage,
    width: u32,
    height: u32,
    slots: &[T],
    candidates: &[RewardCatalogEntry],
    rect: impl Fn(&T) -> Option<(u32, u32, u32, u32)> + Sync + Send,
) -> Vec<Option<SlotRead>>
where
    T: Sync,
{
    let workers = std::thread::available_parallelism()
        .map_or(1, std::num::NonZeroUsize::get)
        .min(slots.len())
        .min(12);
    if workers <= 1 {
        return slots
            .iter()
            .map(|slot| read_slot(luma, image, width, height, rect(slot), candidates))
            .collect();
    }
    let rect = &rect;
    std::thread::scope(|scope| {
        // Round-robin the slots across workers, then weave the results back into order.
        let per_worker: Vec<Vec<usize>> = (0..workers)
            .map(|w| (w..slots.len()).step_by(workers).collect())
            .collect::<Vec<_>>();
        type CropReads = Vec<Option<SlotRead>>;
        let handles: Vec<std::thread::ScopedJoinHandle<'_, CropReads>> = per_worker
            .clone()
            .into_iter()
            .map(|indices| {
                scope.spawn(move || {
                    indices
                        .iter()
                        .map(|&i| {
                            read_slot(luma, image, width, height, rect(&slots[i]), candidates)
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        // Join first (all workers done), then reorder against the owned index lists.
        let mut reads: Vec<Option<SlotRead>> =
            std::iter::repeat_with(|| None).take(slots.len()).collect();
        let chunk_results: Vec<Vec<Option<SlotRead>>> = handles
            .into_iter()
            .map(|handle| handle.join().expect("kiosk ocr worker"))
            .collect();
        for (chunk, indices) in chunk_results.into_iter().zip(per_worker.iter()) {
            for (&i, read) in indices.iter().zip(chunk) {
                reads[i] = read;
            }
        }
        reads
    })
}

/// Luma range below which a band is background rather than text. The game draws labels as
/// bright glyphs on a dark pane, so a populated band swings hard between the two; an empty
/// slot's band barely varies. One cheap pass over the raw band replaces a whole tesseract
/// spawn for every slot the grid is not showing.
const BLANK_BAND_RANGE: u8 = 40;

/// True when the band holds glyphs worth reading.
fn band_has_text(luma: &GrayImage, x: u32, y: u32, w: u32, h: u32) -> bool {
    let mut min = u8::MAX;
    let mut max = u8::MIN;
    for row in y..y + h {
        // `rows` hands back a borrowed slice of one scanline; skip/take trims to the band.
        let stride = luma.width() as usize;
        let start = row as usize * stride + x as usize;
        for cell in &luma.as_raw()[start..start + w as usize] {
            min = min.min(*cell);
            max = max.max(*cell);
        }
    }
    max.saturating_sub(min) >= BLANK_BAND_RANGE
}

fn read_slot(
    luma: &GrayImage,
    image: &DynamicImage,
    width: u32,
    height: u32,
    rect: Option<(u32, u32, u32, u32)>,
    candidates: &[RewardCatalogEntry],
) -> Option<SlotRead> {
    let (x, y, w, h) = rect?;
    // A window smaller than the 1080p calibration cannot contain these fractions at pixel
    // fidelity; clipping to the frame beats panicking on an out-of-bounds view.
    if x + w > width || y + h > height || w == 0 || h == 0 {
        return None;
    }
    if !band_has_text(luma, x, y, w, h) {
        return None;
    }
    let prepared = prepare_crop(image, x, y, w, h);
    let crop = scratch_file();
    prepared.save(&crop).ok()?;
    // Label slots read as one ordered block: the card art above the label is a large bright
    // mass, and sparse-text segmentation lets it eat any glyphs touching it. The basket
    // quantity lane below deliberately stays on `ocr_crop`.
    let text = ocr_crop_block(&crop);
    let _ = std::fs::remove_file(&crop);
    let text = text.ok()?;
    let (name, score) = best_match(&text, candidates)?;
    (score >= MATCH_FLOOR).then_some(SlotRead { name, score })
}

/// Split Warframe's optional basket stack prefix from full-charset band text. The band
/// carries the marker glued to the item name, so the parse looks for digit+X anywhere in
/// the text rather than trusting the first token (the old whitelist lane mangled name
/// initials into counts). Besides `X`, Tesseract's observed `K` confusion is accepted
/// either way. The trailing guard rejects a misread glued to lowercase (`4ksomati...`)
/// while tolerating dropped spaces (`3XBraton`, `2XK`).
fn basket_quantity(text: &str) -> u32 {
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i].is_ascii_digit() {
            let mut j = i;
            while j < bytes.len() && bytes[j].is_ascii_digit() {
                j += 1;
            }
            let mut k = j;
            // Tesseract's band text wraps across lines, so the marker gap is any
            // ASCII whitespace, not just spaces.
            while k < bytes.len() && bytes[k].is_ascii_whitespace() {
                k += 1;
            }
            if k < bytes.len() && matches!(bytes[k], b'X' | b'x' | b'K' | b'k') {
                let after = k + 1;
                if after >= bytes.len() || !bytes[after].is_ascii_lowercase() {
                    if let Ok(count) = text[i..j].parse::<u32>() {
                        return count;
                    }
                }
            }
            i = j;
        } else {
            i += 1;
        }
    }
    1
}

#[cfg(test)]
mod tests {
    use super::*;
    use warframe_acquisition::RewardCatalogEntry;

    /// The catalog subset visible in the fixture, plus near-miss decoys that a garbled read
    /// could plausibly land on if the closed set were doing less work than it should.
    fn candidates() -> Vec<RewardCatalogEntry> {
        [
            "Afentis Prime Blade",
            "Alternox Prime Stock",
            "Athodai Prime Barrel",
            "Atlas Prime Chassis Blueprint",
            "Caliban Prime Systems Blueprint",
            "Epitaph Prime Receiver",
            "Fulmin Prime Receiver",
            "Hystrix Prime Receiver",
            "Euphona Prime Receiver",
            "Titania Prime Systems Blueprint",
            "Tiberon Prime Barrel",
            "Kompressa Prime Barrel",
            "Tiberon Prime Stock",
            "Titania Prime Chassis Blueprint",
        ]
        .into_iter()
        .map(|name| RewardCatalogEntry {
            name: name.to_owned(),
            ducats: 100,
        })
        .collect()
    }

    const FIXTURE: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/kiosk/kiosk-open.png"
    );

    const QUANTITY_LIVE_FIXTURE: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/kiosk/kiosk-quantity-game.png"
    );

    #[test]
    fn reads_known_grid_cells_from_the_fixture() {
        let img = image::open(FIXTURE).unwrap();
        let cells = read_grid(&img, &candidates(), 0);
        for (expected, col, row) in [
            ("Titania Prime Systems Blueprint", 0usize, 0usize),
            ("Tiberon Prime Barrel", 0, 1),
            ("Atlas Prime Chassis Blueprint", 0, 2),
        ] {
            let hit = cells
                .iter()
                .find(|c| c.col == col && c.row == row)
                .unwrap_or_else(|| panic!("cell ({col},{row}) missing; got {cells:?}"));
            assert_eq!(hit.name, expected, "cell ({col},{row})");
            assert!(hit.score >= 0.85, "cell ({col},{row}) score {}", hit.score);
        }
    }

    /// A scrolled grid is read where the rows actually are: shifted one full row pitch down,
    /// row 0's band lands on row 1's labels -- the 2026-08-23 session that died at dy=-142,
    /// had the reads been phase-corrected, would have read its rows exactly like this.
    #[test]
    fn a_scrolled_grid_is_read_at_the_shifted_bands() {
        let img = image::open(FIXTURE).unwrap();
        let cells = read_grid(&img, &candidates(), 222);
        let hit = cells
            .iter()
            .find(|c| c.col == 0 && c.row == 0)
            .expect("row 0 col 0 read through the shifted band");
        assert_eq!(hit.name, "Tiberon Prime Barrel");
    }

    /// At a -142px phase the pane exposes a fourth card row at the bottom. Reuse a known
    /// fixture label there: the reader must enumerate it, not stop at the three unscrolled rows.
    #[test]
    fn a_scrolled_grid_reads_the_fourth_visible_row() {
        use image::GenericImage;

        let source = image::open(FIXTURE).unwrap();
        let mut frame = source.clone();
        let known_label = source.crop_imm(76, 765, 190, 68);
        frame
            .copy_from(&known_label, 76, 845)
            .expect("known label copied into entering row");

        let cells = read_grid(&frame, &candidates(), -142);
        let hit = cells
            .iter()
            .find(|cell| cell.col == 0 && cell.row == 3)
            .unwrap_or_else(|| panic!("fourth visible row missing; got {cells:?}"));
        assert_eq!(hit.name, "Atlas Prime Chassis Blueprint");
    }

    #[test]
    fn parses_stacked_basket_quantity_prefix() {
        assert_eq!(basket_quantity("2 X Kompressa Prime Barrel"), 2);
        assert_eq!(basket_quantity("2 x Kompressa Prime Barrel"), 2);
        // The production threshold pipeline reads the live separator as K/k; retain the
        // deliberately narrow confusion set rather than accepting any token after a number.
        assert_eq!(basket_quantity("2 K Kompressa Prime Barrel"), 2);
        assert_eq!(basket_quantity("2X Kompressa Prime Barrel"), 2);
        assert_eq!(basket_quantity("2k Kompressa Prime Barrel"), 2);
        assert_eq!(basket_quantity("2XK"), 2);
        assert_eq!(basket_quantity("2 k Kompressa Prime Barrel"), 2);
        // PSM-11 band text wraps mid-marker; the gap accepts any whitespace.
        assert_eq!(basket_quantity("2\nX Kompressa Prime Barrel"), 2);
        assert_eq!(basket_quantity("2\tX Kompressa Prime Barrel"), 2);
        assert_eq!(basket_quantity("2 \n X Kompressa Prime Barrel"), 2);
        assert_eq!(basket_quantity("3XBraton Prime Receiver"), 3);
        // Full-charset text can carry junk before a real marker; the marker is still the
        // marker, and junk alone can no longer fabricate one (no whitelist mangling).
        assert_eq!(basket_quantity("kxk 2XK"), 2);
        assert_eq!(basket_quantity("xkx 2XK"), 2);
        assert_eq!(basket_quantity("kxk xkx 2XK"), 2);
        assert_eq!(basket_quantity("Kompressa Prime Barrel"), 1);
        assert_eq!(basket_quantity("2 Kompressa Prime Barrel"), 1);
        assert_eq!(basket_quantity("2 Z Kompressa Prime Barrel"), 1);
        // A name initial glued to a surviving letter is not a stack, and neither are
        // ducat counts or chip prices.
        assert_eq!(basket_quantity("Aksomati Prime Barrel"), 1);
        assert_eq!(basket_quantity("Afuris Prime Link"), 1);
        assert_eq!(basket_quantity("Nekros Prime Systems Blueprint"), 1);
        assert_eq!(basket_quantity("45"), 1);
        assert_eq!(basket_quantity("100"), 1);
        assert_eq!(basket_quantity("135"), 1);
        assert_eq!(basket_quantity("30p"), 1);
        assert_eq!(basket_quantity("4p"), 1);
    }

    #[test]
    fn reads_stacked_quantity_through_production_basket_geometry() {
        let image = image::open(QUANTITY_LIVE_FIXTURE).unwrap();
        let rows = read_basket(&image, &candidates());
        let stacked = rows
            .iter()
            .find(|row| row.name == "Kompressa Prime Barrel")
            .unwrap_or_else(|| panic!("stacked basket row missing; got {rows:?}"));

        assert_eq!(stacked.index, 2);
        assert_eq!(stacked.quantity, 2);
    }

    #[test]
    fn reads_basket_rows_from_the_fixture() {
        let img = image::open(FIXTURE).unwrap();
        let rows = read_basket(&img, &candidates());
        for (expected, index) in [
            ("Afentis Prime Blade", 0usize),
            ("Hystrix Prime Receiver", 6usize),
        ] {
            let hit = rows
                .iter()
                .find(|r| r.index == index)
                .unwrap_or_else(|| panic!("basket row {index} missing; got {rows:?}"));
            assert_eq!(hit.name, expected);
            assert!(hit.score >= 0.85);
        }
    }

    /// The live game basket has three occupied rows. A widened name crop crosses the pane
    /// boundary and can recognize adjacent grid text as a fourth, orphan-priced basket row.
    /// The chip masks blank the pair column where the overlay draws, but the game's own
    /// "2 X" stack marker is a name prefix far left of it: a masked frame must still read a
    /// real stacked quantity.
    #[test]
    fn masking_the_published_chips_does_not_hide_a_stacked_quantity() {
        let mut image = image::open(QUANTITY_LIVE_FIXTURE).unwrap();
        let view = crate::kiosk_view::KioskView {
            session: 0,
            epoch: 0,
            cells: vec![],
            basket: (0..3)
                .map(|index| crate::kiosk_view::BasketChip {
                    index,
                    name: format!("row {index}"),
                    platinum: Some(10),
                })
                .collect(),
            total_plat: 40,
            scroll_dy: 0,
        };
        crate::kiosk_view::mask_published_chips(&mut image, &view, 0);
        let rows = read_basket(&image, &candidates());
        let stacked = rows
            .iter()
            .find(|row| row.name == "Kompressa Prime Barrel")
            .unwrap_or_else(|| panic!("stacked basket row missing post-mask; got {rows:?}"));
        assert_eq!(stacked.index, 2);
        assert_eq!(
            stacked.quantity, 2,
            "the game's own stack marker survives the chip mask: {rows:?}"
        );
    }

    const BASKET16_DEV_FIXTURE: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/kiosk/kiosk-basket-16-dev.png"
    );

    /// The pane fits sixteen rows before the TOTAL row (row 15's baseline is 818 against
    /// TOTAL at 875).
    #[test]
    fn a_sixteen_row_live_basket_reads_through_its_last_row() {
        let image = image::open(BASKET16_DEV_FIXTURE).unwrap();
        let names = [
            "Afuris Prime Link",
            "Atlas Prime Chassis Blueprint",
            "Tiberon Prime Barrel",
            "Vadarya Prime Blueprint",
            "Nekros Prime Systems Blueprint",
            "Panthera Prime Barrel",
            "Titania Prime Systems Blueprint",
            "Trumna Prime Barrel",
            "Trumna Prime Receiver",
            "Wisp Prime Chassis Blueprint",
            "Fragor Prime Handle",
            "Venato Prime Blade",
            "Aksomati Prime Barrel",
            "Alternox Prime Barrel",
            "Akbronco Prime Link",
            "Braton Prime Receiver",
        ];
        let candidates: Vec<RewardCatalogEntry> = names
            .iter()
            .map(|name| RewardCatalogEntry {
                name: (*name).to_owned(),
                ducats: 45,
            })
            .collect();
        let rows = read_basket(&image, &candidates);
        assert_eq!(rows.len(), 16, "all sixteen basket rows read: {rows:?}");
        for (index, name) in names.iter().enumerate() {
            let hit = rows
                .iter()
                .find(|row| row.index == index)
                .unwrap_or_else(|| panic!("basket row {index} missing; got {rows:?}"));
            assert_eq!(&hit.name, name, "basket row {index}");
        }
        for row in &rows {
            let want = if row.index == 15 { 3 } else { 1 };
            assert_eq!(
                row.quantity, want,
                "row {} ({}): no phantom stacks, real marker kept: {rows:?}",
                row.index, row.name
            );
        }
    }

    #[test]
    fn live_basket_does_not_match_adjacent_grid_text_as_a_fourth_row() {
        let image = image::open(QUANTITY_LIVE_FIXTURE).unwrap();
        let rows = read_basket(&image, &candidates());

        assert!(
            rows.iter().all(|row| row.index < 3),
            "basket recognition must stay inside the game basket pane: {rows:?}"
        );
    }
}
