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

use image::{DynamicImage, GenericImageView};
use std::path::PathBuf;

use warframe_acquisition::RewardCatalogEntry;

use crate::{
    kiosk_geometry::basket_label_rect,
    reward_ocr::{best_match, ocr_crop, prepare_crop},
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

/// One recognized basket row.
#[derive(Clone, Debug)]
pub struct BasketRow {
    pub index: usize,
    pub name: String,
    pub score: f32,
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
pub fn read_grid(image: &DynamicImage, candidates: &[RewardCatalogEntry]) -> Vec<GridCell> {
    let (width, height) = image.dimensions();
    let mut cells = Vec::new();
    for row in 0..crate::kiosk_geometry::GRID_ROWS {
        for col in 0..crate::kiosk_geometry::GRID_COLS {
            if let Some(read) = read_slot(
                image,
                width,
                height,
                crate::kiosk_geometry::grid_label_rect(width, height, col, row),
                candidates,
            ) {
                cells.push(GridCell {
                    col,
                    row,
                    name: read.0,
                    score: read.1,
                });
            }
        }
    }
    cells
}

/// Read every visible basket row independently; rows past the list's end read blank and drop out.
pub fn read_basket(image: &DynamicImage, candidates: &[RewardCatalogEntry]) -> Vec<BasketRow> {
    let (width, height) = image.dimensions();
    let mut rows = Vec::new();
    for index in 0..crate::kiosk_geometry::BASKET_ROWS {
        if let Some(read) = read_slot(
            image,
            width,
            height,
            basket_label_rect(width, height, index),
            candidates,
        ) {
            rows.push(BasketRow {
                index,
                name: read.0,
                score: read.1,
            });
        }
    }
    rows
}

fn read_slot(
    image: &DynamicImage,
    width: u32,
    height: u32,
    rect: Option<(u32, u32, u32, u32)>,
    candidates: &[RewardCatalogEntry],
) -> Option<(String, f32)> {
    let (x, y, w, h) = rect?;
    // A window smaller than the 1080p calibration cannot contain these fractions at pixel
    // fidelity; clipping to the frame beats panicking on an out-of-bounds view.
    if x + w > width || y + h > height || w == 0 || h == 0 {
        return None;
    }
    let prepared = prepare_crop(image, x, y, w, h);
    let crop = scratch_file();
    prepared.save(&crop).ok()?;
    let text = ocr_crop(&crop);
    let _ = std::fs::remove_file(&crop);
    let (name, score) = best_match(&text.ok()?, candidates)?;
    (score >= MATCH_FLOOR).then_some((name, score))
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
            "Titania Prime Systems Blueprint",
            "Tiberon Prime Barrel",
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

    #[test]
    fn reads_known_grid_cells_from_the_fixture() {
        let img = image::open(FIXTURE).unwrap();
        let cells = read_grid(&img, &candidates());
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
}
