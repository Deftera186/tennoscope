use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

use app_lib::{
    BasketRow, GridCell, KioskFrameSource, KioskPollerTiming, KioskRead, KioskView,
    spawn_kiosk_poller_with,
};
use warframe_acquisition::RewardCatalogEntry;

/// A scripted source: each call pops the next scripted result; exhaustion reads as a capture
/// error, which drives the poller's miss streak and ends the thread deterministically. A frame
/// may carry the `reanchor` side effect, standing in for the monitor loop's `PopulateGrid()`
/// flag arriving between two reads.
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

fn timing() -> KioskPollerTiming {
    KioskPollerTiming {
        interval: std::time::Duration::from_millis(1),
        motion_interval: std::time::Duration::from_millis(1),
        lifetime: std::time::Duration::from_secs(5),
        grace: std::time::Duration::ZERO,
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

/// Drive the poller over a frame script and collect what it published, plus its flags' final
/// state. The joiner only has to make the frame recognizable in the output: epoch and slot counts.
fn run(frames: Vec<(bool, Result<KioskRead, &'static str>)>) -> RunOutcome {
    run_with_strips(frames, Vec::new())
}

fn run_with_strips(
    frames: Vec<(bool, Result<KioskRead, &'static str>)>,
    strips: Vec<Vec<f32>>,
) -> RunOutcome {
    run_joining(
        frames,
        strips,
        |_epoch, _frame| KioskView::default(),
        |_view| (),
    )
}

struct RunOutcome {
    epochs: Vec<u64>,
    totals: Vec<u64>,
    scrolls: Vec<Option<i32>>,
    gone: bool,
}

fn run_joining(
    frames: Vec<(bool, Result<KioskRead, &'static str>)>,
    strips: Vec<Vec<f32>>,
    join: impl Fn(u64, &KioskRead) -> KioskView + Send + Sync + 'static,
    on_publish: impl Fn(KioskView) + Send + Sync + 'static,
) -> RunOutcome {
    let reanchor = Arc::new(AtomicBool::new(true));
    let gone = Arc::new(AtomicBool::new(false));
    let epochs = Arc::new(Mutex::new(Vec::new()));
    let totals = Arc::new(Mutex::new(Vec::new()));
    let scrolls = Arc::new(Mutex::new(Vec::new()));
    let sink = {
        let epochs = Arc::clone(&epochs);
        let totals = Arc::clone(&totals);
        move |view: KioskView| {
            if let Ok(mut slot) = epochs.lock() {
                slot.push(view.epoch);
            }
            if let Ok(mut slot) = totals.lock() {
                slot.push(view.total_plat);
            }
            on_publish(view);
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
    let joiner = move |epoch: u64, frame: &KioskRead| {
        let mut view = join(epoch, frame);
        view.epoch = epoch;
        view.total_plat = (frame.cells.len() + frame.basket.len()) as u64;
        view
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
        epochs: Arc::try_unwrap(epochs)
            .map(|slot| slot.into_inner().unwrap())
            .unwrap_or_default(),
        totals: Arc::try_unwrap(totals)
            .map(|slot| slot.into_inner().unwrap())
            .unwrap_or_default(),
        scrolls: Arc::try_unwrap(scrolls)
            .map(|slot| slot.into_inner().unwrap())
            .unwrap_or_default(),
        gone: gone.load(Ordering::Acquire),
    }
}

fn plain(frames: Vec<Result<KioskRead, &'static str>>) -> RunOutcome {
    run(frames.into_iter().map(|frame| (false, frame)).collect())
}

/// The poller's day job: read the screen, join it, publish the view. One good frame publishes
/// once; the exhaustion errors afterwards must not publish anything.
#[test]
fn a_good_frame_publishes_once_per_read() {
    let outcome = plain(vec![read(18, 3)]);
    assert_eq!(outcome.totals, vec![21], "cells + basket rode the join");
    assert!(outcome.gone, "exhaustion closed the poller");
}

/// One bad frame -- a capture error or a blank read -- keeps the last published view: an emptying
/// view is worse than a slightly old one, and one miss has never meant the screen closed.
#[test]
fn one_bad_frame_keeps_the_last_view() {
    let outcome = plain(vec![read(18, 3), Err("capture failed"), read(18, 3)]);
    assert_eq!(
        outcome.totals,
        vec![21, 21],
        "only the good frames published"
    );
}

/// Two misses in a row is the gone verdict: EE.log never says the kiosk closed, so the streak is
/// the whole close path.
#[test]
fn two_misses_in_a_row_deliver_the_gone_verdict() {
    let outcome = plain(vec![
        read(18, 3),
        Err("capture failed"),
        Err("capture failed"),
        read(18, 3), // must never be read: the poller exited
    ]);
    assert_eq!(outcome.totals, vec![21]);
    assert!(outcome.gone);
}

/// A hover card covering tiles reads as most of the grid gone. That frame is an occlusion, not an
/// emptied kiosk: nothing publishes, and the miss counts toward the streak exactly like a capture
/// failure.
#[test]
fn a_majority_lost_frame_is_an_occlusion_not_an_emptied_kiosk() {
    let outcome = plain(vec![read(18, 3), read(3, 3), read(18, 3)]);
    assert_eq!(
        outcome.totals,
        vec![21, 21],
        "the 3-cell frame must not publish over the 18-cell anchor"
    );
}

/// An empty basket is a normal state (the player has not picked anything yet) and must publish:
/// only a read with nothing at all is a miss.
#[test]
fn an_empty_basket_still_publishes() {
    let outcome = plain(vec![read(18, 0)]);
    assert_eq!(outcome.totals, vec![18]);
}

/// `PopulateGrid()` (open, filter change, basket edit) re-anchors: the epoch advances so the
/// frontend resets its scroll transform, and the new anchor replaces the old cell count -- a
/// filter that legitimately narrows the grid is not an occlusion.
#[test]
fn a_reanchor_request_advances_the_epoch_and_replaces_the_anchor() {
    // The flag rides the first frame's delivery: it lands between the two reads, the way the
    // monitor loop would set it after seeing PopulateGrid, so the 6-cell grid that follows is a
    // new anchor rather than a lost one.
    let outcome = run(vec![(true, read(18, 3)), (false, read(6, 2))]);
    assert_eq!(outcome.totals, vec![21, 8], "both reads published");
    assert_eq!(
        outcome.epochs,
        vec![1, 2],
        "the epoch advanced on re-anchor"
    );
}

/// A structured strip for the scroll channel: brightness bands of varying height, like rows of
/// thumbnails and labels (same shape as the tracker's own unit tests).
fn bands(len: usize) -> Vec<f32> {
    (0..len)
        .map(|row| {
            let base = match (row / 13) % 5 {
                0 => 20.0,
                1 => 180.0,
                2 => 60.0,
                3 => 200.0,
                _ => 100.0,
            };
            base + (row % 7) as f32
        })
        .collect()
}

/// `new[i] = old[i - dy]` with wrap-around, so lengths always match.
fn shift_rows(rows: &[f32], dy: i32) -> Vec<f32> {
    (0..rows.len())
        .map(|i| rows[(i as i64 - dy as i64).rem_euclid(rows.len() as i64) as usize])
        .collect()
}

/// While the strip is moving, deltas stream at the motion rate and the expensive read waits:
/// the one scripted read serves the initial anchor, and the moving strip must not consume
/// another.
#[test]
fn motion_streams_deltas_and_defers_the_full_read() {
    let base = bands(400);
    let outcome = run_with_strips(
        vec![(false, read(18, 3))],
        vec![base.clone(), shift_rows(&base, -23)],
    );
    assert_eq!(outcome.totals, vec![21], "only the anchor's read published");
    assert_eq!(outcome.scrolls, vec![Some(-23)], "the motion streamed");
}

/// A settle after motion re-anchors: the epoch advances so the frontend drops the transform the
/// deltas accumulated, and the fresh read publishes against the new epoch.
#[test]
fn a_settle_after_motion_reanchors_with_a_fresh_epoch() {
    let base = bands(400);
    let scrolled = shift_rows(&base, -31);
    let mut strips = vec![base, scrolled.clone()];
    // still looks until SETTLE_TICKS is satisfied, then the settle's look
    for _ in 0..4 {
        strips.push(scrolled.clone());
    }
    let outcome = run_with_strips(vec![(false, read(18, 3)), (false, read(18, 3))], strips);
    assert_eq!(outcome.totals, vec![21, 21]);
    assert_eq!(outcome.epochs, vec![1, 2], "the settle bumped the epoch");
    assert_eq!(
        outcome.scrolls,
        vec![Some(-31), Some(0), Some(0), Some(0)],
        "three still looks precede the settle (the fourth settles)"
    );
}

/// A strip that cannot be measured fades the chips (`None`) and counts a miss like a failed
/// read. Blindness also drops the tracker's anchor, so the look after it is a forced full read
/// -- here scripted to fail, which is what a screen that has gone blank reads like, and two
/// misses close the session with the last good view standing.
#[test]
fn a_blind_strip_fades_then_a_failed_reanchor_closes() {
    let outcome = run_with_strips(
        // One good read for the anchor; the post-blind re-anchor read fails (script exhausted).
        vec![(false, read(18, 3))],
        vec![bands(400), vec![90.0; 400], vec![90.0; 400]],
    );
    assert_eq!(outcome.totals, vec![21], "the anchor stands");
    assert_eq!(outcome.scrolls, vec![None], "the blind look faded");
    assert!(
        outcome.gone,
        "the blind miss plus the failed read closed it"
    );
}

/// Motion that never settles is not our grid forever: after the cap, a full read is forced,
/// which either re-anchors (here it does) or starts the streak that closes.
#[test]
fn unending_motion_is_capped_by_a_forced_read() {
    let base = bands(400);
    let scrolling = shift_rows(&base, -3);
    // anchor look, then endless motion looks
    let mut strips = vec![base];
    for _ in 0..600 {
        strips.push(scrolling.clone());
    }
    let outcome = run_with_strips(vec![(false, read(18, 3)), (false, read(18, 3))], strips);
    assert_eq!(
        outcome.totals.len(),
        2,
        "the forced read after the motion cap published"
    );
}
