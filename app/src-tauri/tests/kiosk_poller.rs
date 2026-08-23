//! The kiosk poller's contract, played against a scripted screen: what streams while the grid
//! moves, what publishes when it stops, and how it stops. What it does NOT decide is presence:
//! EE.log owns that (see `kiosk_wiring`), so no failure in here may end a session.

use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

use app_lib::{
    BasketRow, GridCell, KioskFrameSource, KioskPollerTiming, KioskRead, KioskView,
    spawn_kiosk_poller_with,
};
use warframe_acquisition::RewardCatalogEntry;

/// A scripted source: strips are popped one per look, frames one per read; exhaustion reads as
/// a capture error, which costs that look and nothing else -- the run simply ends at its
/// lifetime. A frame may carry the `reanchor` side effect, standing in for the monitor loop's
/// `PopulateGrid()` flag arriving between two reads.
struct ScriptedKiosk {
    frames: Mutex<Vec<(bool, Result<KioskRead, &'static str>)>>,
    strips: Mutex<Vec<Vec<f32>>>,
    reanchor: Option<Arc<AtomicBool>>,
}

impl ScriptedKiosk {
    fn scripted(frames: Vec<(bool, Result<KioskRead, &'static str>)>) -> Self {
        // Popped from the tail, so reverse once here and `pop` reads in script order.
        Self {
            frames: Mutex::new(frames.into_iter().rev().collect()),
            strips: Mutex::new(Vec::new()),
            reanchor: None,
        }
    }
}

impl KioskFrameSource for ScriptedKiosk {
    fn strip_profile(&mut self) -> Result<Vec<f32>, &'static str> {
        self.strips.lock().unwrap().pop().ok_or("strip exhausted")
    }

    fn read_kiosk(
        &mut self,
        _candidates: &[RewardCatalogEntry],
        _dy: i32,
    ) -> Result<KioskRead, &'static str> {
        let (flag, frame) = self
            .frames
            .lock()
            .unwrap()
            .pop()
            .unwrap_or((false, Err("script exhausted")));
        if flag && let Some(reanchor) = &self.reanchor {
            reanchor.store(true, Ordering::Release);
        }
        frame
    }
}

fn read(cells: usize, basket: usize) -> Result<KioskRead, &'static str> {
    Ok(KioskRead {
        cells: (0..cells)
            .map(|slot| GridCell {
                col: slot % 6,
                row: slot / 6,
                name: format!("Item {slot}"),
                score: 0.9,
            })
            .collect(),
        basket: (0..basket)
            .map(|index| BasketRow {
                index,
                name: format!("Item {index}"),
                score: 0.9,
            })
            .collect(),
    })
}

/// A short lifetime on purpose: with no close verdict of its own, a session runs until the
/// lifetime ends, and 400ms at a 1ms tick is dozens of looks -- plenty for every script here,
/// and it keeps the tests deterministic instead of script-length-dependent.
fn timing() -> KioskPollerTiming {
    KioskPollerTiming {
        interval: std::time::Duration::from_millis(1),
        motion_interval: std::time::Duration::from_millis(1),
        lifetime: std::time::Duration::from_millis(400),
    }
}

fn candidates() -> Arc<Vec<RewardCatalogEntry>> {
    Arc::new(
        (0..24)
            .map(|slot| RewardCatalogEntry {
                name: format!("Item {slot}"),
                ducats: 45,
            })
            .collect(),
    )
}

/// A pane profile at 1080p scale: 790 rows (the whole pane), bands every 222 with two text
/// lines each, the first band at strip row 150 -- which `label_offset` reads as offset zero.
/// A count badge is staged above the first label: bright enough to see, far too weak and
/// narrow to be mistaken for a label window.
fn label_strip_at(dy: i64) -> Vec<f32> {
    let mut rows = vec![2.0_f32; 790];
    let mut top = 150 + dy;
    while top < 790 {
        if top >= 0 {
            // Bands clip at the pane's bottom edge exactly as the game clips them.
            let first_end = (top + 11).min(790);
            for row in &mut rows[top as usize..first_end as usize] {
                *row = 250.0;
            }
            let second = top + 27;
            if second + 11 <= 790 {
                for row in &mut rows[second as usize..second as usize + 11] {
                    *row = 240.0;
                }
            }
        }
        top += 222;
    }
    let badge = 23 + dy;
    if badge > 0 && badge + 14 <= 790 {
        for row in &mut rows[badge as usize..badge as usize + 14] {
            *row = 30.0;
        }
    }
    rows
}

fn label_strip() -> Vec<f32> {
    label_strip_at(0)
}

/// A strip with nothing white in it: an animation frame, or a filter that emptied the grid.
fn bare_strip() -> Vec<f32> {
    vec![2.0_f32; 790]
}

/// Everything the poller did over one scripted run.
struct RunOutcome {
    epochs: Vec<u64>,
    totals: Vec<u64>,
    scrolls: Vec<Option<i32>>,
    dys: Vec<i32>,
    gone: bool,
}

/// Drive the poller over a frame script plus a strip script and collect everything it did.
/// The joiner only has to make the frame recognizable in the output: epoch and slot counts.
fn run_with_strips(
    frames: Vec<(bool, Result<KioskRead, &'static str>)>,
    strips: Vec<Vec<f32>>,
) -> RunOutcome {
    let reanchor = Arc::new(AtomicBool::new(true));
    let gone = Arc::new(AtomicBool::new(false));
    let epochs = Arc::new(Mutex::new(Vec::new()));
    let totals = Arc::new(Mutex::new(Vec::new()));
    let scrolls = Arc::new(Mutex::new(Vec::new()));
    let dys = Arc::new(Mutex::new(Vec::new()));
    let sink = {
        let epochs = Arc::clone(&epochs);
        let totals = Arc::clone(&totals);
        let dys = Arc::clone(&dys);
        move |view: KioskView| {
            if let Ok(mut slot) = epochs.lock() {
                slot.push(view.epoch);
            }
            if let Ok(mut slot) = totals.lock() {
                slot.push(view.total_plat);
            }
            if let Ok(mut slot) = dys.lock() {
                slot.push(view.scroll_dy);
            }
        }
    };
    let scroll_sink = {
        let scrolls = Arc::clone(&scrolls);
        move |dy: Option<i32>| {
            if let Ok(mut slot) = scrolls.lock() {
                slot.push(dy);
            }
        }
    };
    let joiner = move |epoch: u64, frame: &KioskRead| KioskView {
        epoch,
        total_plat: (frame.cells.len() + frame.basket.len()) as u64,
        ..KioskView::default()
    };
    let reanchor_arg = Arc::clone(&reanchor);
    let make_source = move || {
        let mut source = ScriptedKiosk::scripted(frames);
        source.strips = Mutex::new(strips.into_iter().rev().collect());
        source.reanchor = Some(reanchor_arg);
        source
    };
    let gone_arg = Arc::clone(&gone);
    spawn_kiosk_poller_with(
        &reanchor,
        &gone_arg,
        timing(),
        candidates(),
        joiner,
        sink,
        scroll_sink,
        make_source,
    )
    .join()
    .expect("poller thread panicked");
    RunOutcome {
        epochs: epochs.lock().unwrap().clone(),
        totals: totals.lock().unwrap().clone(),
        scrolls: scrolls.lock().unwrap().clone(),
        dys: dys.lock().unwrap().clone(),
        gone: gone.load(Ordering::Acquire),
    }
}

/// The poller's day job: two still looks settle it, the read runs where the labels say the
/// rows are, the view publishes. Script exhaustion afterwards is just failed looks -- the run
/// ends at its lifetime, and the flags show no verdict, because the poller has none to give.
#[test]
fn a_good_frame_publishes_once_per_read() {
    let outcome = run_with_strips(vec![(false, read(18, 3))], vec![label_strip(); 4]);
    assert_eq!(outcome.totals, vec![21], "cells + basket rode the join");
    assert!(!outcome.gone, "an exhausted script is not a close");
}

/// One bad read keeps the last published view: an emptying view is worse than a slightly old
/// one, and one miss has never meant anything at all -- the session just tries again.
#[test]
fn one_bad_frame_keeps_the_last_view() {
    let outcome = run_with_strips(
        vec![
            (false, read(18, 3)),
            (false, Err("capture failed")),
            (false, read(18, 3)),
        ],
        vec![label_strip(); 8],
    );
    assert_eq!(
        outcome.totals,
        vec![21, 21],
        "only the good reads published"
    );
    assert!(!outcome.gone);
}

/// Presence is EE.log's answer, not the reader's. A pane with no labels in it -- an
/// animation, a filter that matched nothing, a torn capture -- is a look the poller skips,
/// and the session goes on. (Reading this as a close tore the overlay down mid-session all
/// through 2026-08-23.)
#[test]
fn an_unreadable_pane_is_a_skipped_look_not_a_close() {
    let outcome = run_with_strips(vec![(false, read(18, 3))], vec![bare_strip(); 20]);
    assert!(!outcome.gone);
    assert!(
        outcome.totals.is_empty(),
        "nothing was published over nothing"
    );
}

/// An empty basket is a normal state (the player has not picked anything yet) and must publish:
/// only a read with nothing at all is a miss.
#[test]
fn an_empty_basket_still_publishes() {
    let outcome = run_with_strips(vec![(false, read(18, 0))], vec![label_strip(); 4]);
    assert_eq!(outcome.totals, vec![18]);
}

/// `PopulateGrid()` (open, filter change, basket edit) re-anchors: the epoch advances so the
/// frontend resets its scroll transform.
#[test]
fn a_reanchor_request_advances_the_epoch() {
    let outcome = run_with_strips(
        vec![(true, read(18, 3)), (false, read(6, 2))],
        vec![label_strip(); 8],
    );
    assert_eq!(outcome.totals, vec![21, 8], "both reads published");
    assert_eq!(
        outcome.epochs,
        vec![1, 2],
        "the epoch advanced on re-anchor"
    );
}

/// Measurable motion streams frame-to-frame deltas instead of fading, and no recognition pass
/// runs while it lasts -- a mid-scroll read catches rows straddling the bands and comes back
/// empty, which is how sessions used to die.
#[test]
fn motion_streams_deltas_and_defers_the_read() {
    let outcome = run_with_strips(
        vec![(false, read(18, 3))],
        vec![
            label_strip(),
            label_strip_at(-23),
            label_strip_at(-40),
            label_strip_at(-40),
            label_strip_at(-40),
            label_strip_at(-40),
        ],
    );
    assert_eq!(outcome.totals, vec![21], "only the settled read published");
    assert_eq!(
        &outcome.scrolls[..2],
        &[Some(-23), Some(-17)],
        "deltas stream frame to frame: {:?}",
        outcome.scrolls
    );
}

/// A scroll that stops anywhere settles into a read located by the grid's own label rows, and
/// the view carries that offset: the chips land on the rows wherever the scroll stopped.
/// (2026-08-23's session stopped at -142, read the calibration gaps, and died.)
#[test]
fn a_stopped_scroll_publishes_the_located_offset() {
    let outcome = run_with_strips(
        vec![(false, read(18, 3)), (false, read(18, 3))],
        vec![
            label_strip(),
            label_strip(),
            label_strip_at(-142),
            label_strip_at(-142),
            label_strip_at(-142),
            label_strip_at(-142),
        ],
    );
    assert_eq!(outcome.totals, vec![21, 21], "anchor and settle published");
    // Flat blocks give the locator no finer answer than their containment plateau (see
    // below), so the anchor read too is only bound to sit over the band's text -- which is
    // all the OCR crop and the chip translate need from it.
    let anchor_drift = outcome.dys[0].rem_euclid(222);
    assert!(
        anchor_drift == 0 || anchor_drift + 8 >= 222,
        "the anchor read sits over the band: {:?}",
        outcome.dys
    );
    // The view carries the label phase. These synthetic bands are flat blocks, so the
    // locator's containment plateau is at its widest: anywhere that keeps row 0's crop over
    // the anchor band's whole text (within 8 rows above its top) is a correct answer.
    let drift = (outcome.dys[1] + 142).rem_euclid(222);
    assert!(
        drift == 0 || drift + 8 >= 222,
        "the view carries the phase: {:?}",
        outcome.dys
    );
}

/// Motion that never pauses is still our grid: every look measures confidently, so the poller
/// follows it for as long as it lasts and never spends a recognition pass.
#[test]
fn sustained_motion_never_reads() {
    let mut strips = vec![label_strip()];
    // A triangle wave: every look moves another 3 rows from the last (never a pause).
    for k in 0..30 {
        let pos = k % 28;
        let shift = if pos < 14 {
            3 * (pos + 1)
        } else {
            3 * (28 - pos)
        };
        strips.push(label_strip_at(-shift));
    }
    let outcome = run_with_strips(vec![(false, read(18, 3))], strips);
    assert!(outcome.totals.is_empty(), "no mid-scroll read ever ran");
    assert!(
        outcome.scrolls.len() >= 20 && outcome.scrolls[..20].iter().all(|v| v.is_some()),
        "the scroll streamed, no fades: {:?}",
        &outcome.scrolls[..20.min(outcome.scrolls.len())]
    );
}

/// An empty settle read that raced a resumed scroll is not the kiosk being gone: the poller
/// has no misses to advance anymore -- it just keeps looking, streaming while the grid moves,
/// and outlives the empty read to settle properly later.
#[test]
fn an_empty_settle_read_while_the_grid_moves_is_not_the_kiosk_gone() {
    // Two still looks settle into the good read; then a still look settles into the empty
    // one; then the grid keeps moving for many looks -- deltas stream throughout and the
    // session goes on.
    let mut strips = vec![
        label_strip(),
        label_strip(),
        label_strip_at(-9),
        label_strip_at(-9),
        label_strip_at(-9),
    ];
    for k in 0..25 {
        strips.push(label_strip_at(-9 - 3 * (k % 8) as i64));
    }
    let outcome = run_with_strips(vec![(false, read(18, 3)), (false, read(0, 0))], strips);
    assert!(
        outcome.scrolls.len() >= 15,
        "the session kept looking long after the empty settle read: {:?}",
        outcome.scrolls.len()
    );
    assert!(!outcome.gone);
}

/// The external verdict -- the monitor acting on EE.log's close line -- is the only thing
/// that stops the poller besides its lifetime. Once the shared `gone` flag is set from
/// outside, the poller stops looking at a kiosk the game has already hidden: no further
/// reads, no further publishes.
#[test]
fn the_external_close_verdict_stops_the_poller() {
    // Two still looks anchor the view; then the grid never stops moving, so only the
    // external verdict can end the poller before its lifetime.
    let mut strips = vec![label_strip(), label_strip()];
    strips.extend((0..200).map(|k| label_strip_at(-3 * (k % 20) as i64)));
    let frames = vec![
        (false, read(18, 3)),
        (false, read(6, 2)), // must never be read
    ];
    let reanchor = Arc::new(AtomicBool::new(true));
    let gone = Arc::new(AtomicBool::new(false));
    let totals = Arc::new(Mutex::new(Vec::new()));
    let totals_sink = Arc::clone(&totals);
    let gone_arg = Arc::clone(&gone);
    let handle = spawn_kiosk_poller_with(
        &reanchor,
        &gone_arg,
        timing(),
        candidates(),
        |_epoch, _frame| KioskView::default(),
        move |view: KioskView| {
            if let Ok(mut slot) = totals_sink.lock() {
                slot.push(view.total_plat);
            }
        },
        |_| (),
        move || {
            let mut source = ScriptedKiosk::scripted(frames);
            // Popped from the tail, like every scripted run.
            source.strips = Mutex::new(strips.into_iter().rev().collect());
            source
        },
    );
    // The anchor publishes; then the external verdict lands.
    std::thread::sleep(std::time::Duration::from_millis(30));
    gone_arg.store(true, Ordering::Release);
    handle.join().expect("poller thread panicked");
    assert_eq!(
        totals.lock().unwrap().len(),
        1,
        "the anchor published once, then the external verdict stopped the poller"
    );
}
