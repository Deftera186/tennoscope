//! Drive the deep reward-recognition module through its public contract.
//!
//! The old poller and relic-pool suites exercised `ScreenWatch`/`RelicPool` machinery that
//! Main removes with this cutover; their live-fissure regressions are ported here against
//! `RewardRecognition` with the same scripted-screen technique, now driven by channels and
//! barriers, and any assertion failure still lets the worker finish. One bounded 80ms sleep
//! remains for the empty-pool case, as deadlock protection while the worker idles.
//!
//! Ownership rules this file relies on (from the shared plan):
//! - the worker constructs, reuses and drops its own capture adapter (never `Send`), one per
//!   open epoch, and stays idle without one between screens;
//! - `close` is nonblocking and invalidates queued/in-flight results; `shutdown`/`Drop` join
//!   and wait for in-flight capture destruction;
//! - a delivery only publishes while its epoch and evidence revision are still current;
//! - `ChoicesReady`'s count outranks `ResponsesComplete`'s roster size;
//! - count/local constraints known before publication are enforced; the first 2-4 card read
//!   may publish before delayed log constraints arrive;
//! - two consecutive misses after a found screen close it; one blank never flickers it;
//! - the initial overlay publication runs once per epoch through `RewardPublication`
//!   (delayed market effects use the monitor generation guard instead), and a duplicate
//!   or retired token declines.
//!
//! The new regression `stopping_discards_an_undelivered_reward_read` failed on the original
//! queued-read code; `close_discards_an_undelivered_reward_read` keeps equivalent behaviour:
//! a screen that is closed while a result is pending never delivers it.

use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

use app_lib::reward_recognition::{RecognitionTiming, RecognizedRewards, RewardRecognition};
use app_lib::{RewardLogEvent, VisualRewardSource};
use warframe_acquisition::{CatalogIndex, RelicRewardIndex, RewardCatalogEntry};

// ---------------------------------------------------------------------------
// Fixtures: real WFCD catalog + relic-index JSON, so candidate provenance,
// pool sizing and StoreItems-vs-Types path matching come from the real codecs.
// ---------------------------------------------------------------------------

const CATALOG_JSON: &str = r#"[
  {"uniqueName":"/Lotus/Weapons/Tenno/Primary/PerigalePrime","name":"Perigale Prime","type":"Primary","category":"Primary","masterable":true,"components":[
    {"uniqueName":"/Lotus/Types/Recipes/Weapons/PerigalePrimeBarrelComponent","name":"Barrel","tradable":true,"ducats":15,"primeSellingPrice":15,"imageName":"PerigalePrimeBarrel.png"},
    {"uniqueName":"/Lotus/Types/Recipes/Weapons/PerigalePrimeReceiverComponent","name":"Receiver","tradable":true,"ducats":15,"primeSellingPrice":15,"imageName":"PerigalePrimeReceiver.png"},
    {"uniqueName":"/Lotus/Types/Recipes/Weapons/PerigalePrimeStockComponent","name":"Stock","tradable":true,"ducats":15,"primeSellingPrice":15,"imageName":"PerigalePrimeStock.png"}
  ]},
  {"uniqueName":"/Lotus/Weapons/Tenno/Primary/BurstonPrime","name":"Burston Prime","type":"Primary","category":"Primary","masterable":true,"components":[
    {"uniqueName":"/Lotus/Types/Recipes/Weapons/BurstonPrimeBarrelComponent","name":"Barrel","tradable":true,"ducats":15,"primeSellingPrice":15,"imageName":"BurstonPrimeBarrel.png"},
    {"uniqueName":"/Lotus/Types/Recipes/Weapons/BurstonPrimeReceiverComponent","name":"Receiver","tradable":true,"ducats":15,"primeSellingPrice":15,"imageName":"BurstonPrimeReceiver.png"},
    {"uniqueName":"/Lotus/Types/Recipes/Weapons/BurstonPrimeStockComponent","name":"Stock","tradable":true,"ducats":15,"primeSellingPrice":15,"imageName":"BurstonPrimeStock.png"}
  ]},
  {"uniqueName":"/Lotus/Weapons/Tenno/Primary/VastoPrime","name":"Vasto Prime","type":"Primary","category":"Primary","masterable":true,"components":[
    {"uniqueName":"/Lotus/Types/Recipes/Weapons/VastoPrimeBarrelComponent","name":"Barrel","tradable":true,"ducats":15,"primeSellingPrice":15,"imageName":"VastoPrimeBarrel.png"},
    {"uniqueName":"/Lotus/Types/Recipes/Weapons/VastoPrimeReceiverComponent","name":"Receiver","tradable":true,"ducats":15,"primeSellingPrice":15,"imageName":"VastoPrimeReceiver.png"},
    {"uniqueName":"/Lotus/Types/Recipes/Weapons/VastoPrimeStockComponent","name":"Stock","tradable":true,"ducats":15,"primeSellingPrice":15,"imageName":"VastoPrimeStock.png"}
  ]},
  {"uniqueName":"/Lotus/Powersuits/BrokenFrame/XakuPrime","name":"Xaku Prime","type":"Warframe","category":"Warframes","masterable":true,"components":[
    {"uniqueName":"/Lotus/Types/Recipes/Warframes/XakuPrimeHelmetComponent","name":"Neuroptics","tradable":true,"ducats":15,"primeSellingPrice":15,"imageName":"XakuPrimeHelmet.png"}
  ]}
]"#;

const RELICS_JSON: &str = r#"[
  {"uniqueName":"/Lotus/Types/Game/Projections/T1VoidProjectionEarlyBronze","name":"Lith E1 Intact","rewards":[
    {"item":{"name":"Perigale Prime Receiver"}},
    {"item":{"name":"Vasto Prime Stock"}}
  ]},
  {"uniqueName":"/Lotus/Types/Game/Projections/T1VoidProjectionGrownBronze","name":"Lith G1 Intact","rewards":[
    {"item":{"name":"Burston Prime Receiver"}},
    {"item":{"name":"Forma Blueprint"}}
  ]},
  {"uniqueName":"/Lotus/Types/Game/Projections/T1VoidProjectionLaterBronze","name":"Lith L1 Intact","rewards":[
    {"item":{"name":"Burston Prime Receiver"}},
    {"item":{"name":"Xaku Prime Neuroptics Blueprint"}}
  ]}
]"#;

const EARLY_RELIC: &str = "/Lotus/Types/Game/Projections/T1VoidProjectionEarlyBronze";
const GROWN_RELIC: &str = "/Lotus/Types/Game/Projections/T1VoidProjectionGrownBronze";
const LATER_RELIC: &str = "/Lotus/Types/Game/Projections/T1VoidProjectionLaterBronze";

fn catalog() -> CatalogIndex {
    CatalogIndex::from_wfcd_json(CATALOG_JSON.as_bytes()).expect("catalog fixture parses")
}

fn relics() -> RelicRewardIndex {
    RelicRewardIndex::from_wfcd_json(RELICS_JSON.as_bytes()).expect("relic fixture parses")
}

fn rewards() -> Vec<RewardCatalogEntry> {
    catalog().reward_entries()
}

/// Fast everywhere, so a test measures behaviour rather than wall-clock.
fn timing() -> RecognitionTiming {
    RecognitionTiming {
        interval: Duration::from_millis(25),
        watch_interval: Duration::from_millis(25),
        lifetime: Duration::from_secs(5),
        retry_interval: Duration::from_millis(20),
        retry_deadline: Duration::from_millis(40),
    }
}

fn names(entries: &[RewardCatalogEntry]) -> Vec<String> {
    entries.iter().map(|entry| entry.name.clone()).collect()
}

fn entry(name: &str) -> RewardCatalogEntry {
    RewardCatalogEntry {
        name: name.to_owned(),
        ducats: 15,
    }
}

fn baseline(paths: &[&str]) -> RewardLogEvent {
    RewardLogEvent::BaselineRequested {
        relic_paths: paths.iter().map(|path| (*path).to_owned()).collect(),
    }
}

fn choices_ready(expected: usize, local: Option<&str>) -> RewardLogEvent {
    RewardLogEvent::ChoicesReady {
        expected_choices: expected,
        local_reward_path: local.map(str::to_owned),
    }
}

// ---------------------------------------------------------------------------
// Scripted capture adapter. The worker makes one instance per open epoch; the
// test thread sees each capture start, gates its release, and is told when the
// instance is dropped -- so no sleep is ever needed to catch a race.
// ---------------------------------------------------------------------------

enum Frame {
    Names(Vec<String>),
    Blank,
    /// Signal start, then wait on the gate before returning `frame`.
    Gated(Box<Frame>),
}

struct Coordination {
    script: Mutex<Vec<Frame>>,
    capture_started: Mutex<mpsc::Sender<()>>,
    /// Gates consumed in order: each gated frame blocks on the next receiver
    /// the test stacked, so a script can hold the worker mid-flight more than
    /// once.
    gate_stack: Mutex<Vec<mpsc::Receiver<()>>>,
    dropped: Mutex<mpsc::Sender<usize>>,
    captures: AtomicUsize,
    made: AtomicUsize,
}

struct ScriptedSource {
    coordination: Arc<Coordination>,
    drop_signal: Option<mpsc::Sender<usize>>,
}

impl ScriptedSource {
    /// Pop the next scripted frame; a leftover `Names`/`Blank` frame repeats so
    /// a screen reads the same cards until the script moves on. Gated frames
    /// are one-shot: the gate is what paces them.
    fn next_frame(&self) -> Option<Frame> {
        let mut script = self.coordination.script.lock().expect("script lock");
        if script.is_empty() {
            return None;
        }
        if script.len() == 1 && !matches!(script[0], Frame::Gated(_)) {
            match &script[0] {
                Frame::Names(names) => {
                    let names = names.clone();
                    drop(script);
                    return Some(Frame::Names(names));
                }
                Frame::Blank => {
                    drop(script);
                    return Some(Frame::Blank);
                }
                Frame::Gated(_) => unreachable!(),
            }
        }
        Some(script.remove(0))
    }
}

impl VisualRewardSource for ScriptedSource {
    fn choices(&mut self, _candidates: &[RewardCatalogEntry]) -> Result<Vec<String>, &'static str> {
        self.coordination.captures.fetch_add(1, Ordering::AcqRel);
        // The test thread reads `captures` and gates; the signal is a hint it
        // may have already left during teardown, so a missing listener is fine.
        let _ = self
            .coordination
            .capture_started
            .lock()
            .expect("capture signal lock")
            .send(());
        match self.next_frame() {
            Some(Frame::Gated(frame)) => {
                let gate = self
                    .coordination
                    .gate_stack
                    .lock()
                    .expect("gate lock")
                    .drain(..1)
                    .next()
                    .expect("a gated frame requires a gate");
                let _ = gate.recv(); // block until the test releases the guard
                self.frame_result(*frame)
            }
            Some(frame) => self.frame_result(frame),
            None => Err("a reward card read as blank"),
        }
    }
}

impl ScriptedSource {
    fn frame_result(&mut self, frame: Frame) -> Result<Vec<String>, &'static str> {
        match frame {
            Frame::Names(names) => Ok(names),
            Frame::Blank => Err("a reward card read as blank"),
            Frame::Gated(_) => unreachable!("Gated frames are unwrapped before frame_result"),
        }
    }
}

impl Drop for ScriptedSource {
    fn drop(&mut self) {
        let _ = self
            .drop_signal
            .take()
            .and_then(|signal| signal.send(1).ok());
    }
}

/// Auto-releases a worker capture whose gate is held, so an assertion failure
/// in the test body cannot leave the worker blocked at Drop.
struct GateGuard(Option<mpsc::Sender<()>>);

impl GateGuard {
    fn release(&mut self) {
        if let Some(sender) = self.0.take() {
            let _ = sender.send(());
        }
    }
}

impl Drop for GateGuard {
    fn drop(&mut self) {
        self.release();
    }
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

fn coordination(script: Vec<Frame>) -> Arc<Coordination> {
    let (capture_started_tx, _) = mpsc::channel();
    Arc::new(Coordination {
        script: Mutex::new(script),
        capture_started: Mutex::new(capture_started_tx),
        gate_stack: Mutex::new(Vec::new()),
        dropped: Mutex::new(mpsc::channel().0),
        captures: AtomicUsize::new(0),
        made: AtomicUsize::new(0),
    })
}

impl Coordination {
    /// Stack a receiver the next gated frame will block on. Returns the sender
    /// half so a test can build a `GateGuard` from it; the worker's `recv`
    /// unblocks when the guard is released (or dropped during a panic).
    fn stack_gate(&self) -> mpsc::Sender<()> {
        let (gate_tx, gate_rx) = mpsc::channel();
        self.gate_stack.lock().expect("gate lock").push(gate_rx);
        gate_tx
    }
}

fn recognition(
    coordination: &Arc<Coordination>,
) -> RewardRecognition<ScriptedSource, impl Fn() -> ScriptedSource + Send + Sync + 'static> {
    let coordination_for_factory = Arc::clone(coordination);
    RewardRecognition::new(
        Some(catalog()),
        Some(relics()),
        rewards(),
        timing(),
        move || {
            coordination_for_factory.made.fetch_add(1, Ordering::AcqRel);
            ScriptedSource {
                coordination: Arc::clone(&coordination_for_factory),
                drop_signal: Some(
                    coordination_for_factory
                        .dropped
                        .lock()
                        .expect("drop lock")
                        .clone(),
                ),
            }
        },
    )
}

fn wait_for_capture(coordination: &Coordination, expected: usize) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while coordination.captures.load(Ordering::Acquire) < expected {
        if Instant::now() >= deadline {
            panic!("capture #{} never started", expected);
        }
        std::thread::yield_now();
    }
}

/// Poll until the next recognized set or five seconds of deadlock protection.
fn drain_recognized(
    recognition: &mut RewardRecognition<
        ScriptedSource,
        impl Fn() -> ScriptedSource + Send + Sync + 'static,
    >,
) -> RecognizedRewards {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let update = recognition.poll();
        if let Some(recognized) = update.recognized {
            return recognized;
        }
        if Instant::now() >= deadline {
            panic!("no recognized reward set");
        }
        std::thread::yield_now();
    }
}

/// Poll for five seconds of deadlock protection, asserting nothing publishes.
fn drain_and_assert_nothing_recognized(
    recognition: &mut RewardRecognition<
        ScriptedSource,
        impl Fn() -> ScriptedSource + Send + Sync + 'static,
    >,
) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        let update = recognition.poll();
        assert!(
            update.recognized.is_none(),
            "a retired/stale screen must not publish",
        );
        std::thread::yield_now();
    }
}
// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// The whole point of the reader: a 2, 3 or 4 card read publishes with no
/// log-triggered constraints yet known, and the elapsed time is real capture
/// time rather than a fabricated zero.
#[test]
fn a_read_of_two_three_or_four_cards_publishes_without_log_constraints() {
    for (frame, expected) in [
        (
            Frame::Names(names(&[
                entry("Perigale Prime Receiver"),
                entry("Vasto Prime Stock"),
            ])),
            names(&[entry("Perigale Prime Receiver"), entry("Vasto Prime Stock")]),
        ),
        (
            Frame::Names(names(&[
                entry("Perigale Prime Receiver"),
                entry("Vasto Prime Stock"),
                entry("Burston Prime Receiver"),
            ])),
            names(&[
                entry("Perigale Prime Receiver"),
                entry("Vasto Prime Stock"),
                entry("Burston Prime Receiver"),
            ]),
        ),
        (
            Frame::Names(names(&[
                entry("Perigale Prime Receiver"),
                entry("Vasto Prime Stock"),
                entry("Burston Prime Receiver"),
                entry("Forma Blueprint"),
            ])),
            names(&[
                entry("Perigale Prime Receiver"),
                entry("Vasto Prime Stock"),
                entry("Burston Prime Receiver"),
                entry("Forma Blueprint"),
            ]),
        ),
    ] {
        let coordination = coordination(vec![frame]);
        let mut recognition = recognition(&coordination);
        recognition.observe(&baseline(&[EARLY_RELIC]));
        let update = drain_recognized(&mut recognition);
        assert_eq!(update.names, expected);
        assert!(
            update.elapsed < Duration::from_secs(2),
            "elapsed must be the real capture time, not fabricated"
        );
        recognition.shutdown();
    }
}

/// The log announces the rewards before Warframe paints the cards, so the first capture reads
/// an empty screen. Retry until the cards exist rather than giving up on the first blank read.
#[test]
fn a_blank_first_read_retries_until_the_cards_are_painted() {
    let coordination = coordination(vec![
        Frame::Blank,
        Frame::Names(names(&[
            entry("Perigale Prime Receiver"),
            entry("Vasto Prime Stock"),
        ])),
    ]);
    let mut recognition = recognition(&coordination);
    recognition.observe(&baseline(&[EARLY_RELIC]));
    let update = drain_recognized(&mut recognition);
    assert_eq!(
        update.names,
        names(&[entry("Perigale Prime Receiver"), entry("Vasto Prime Stock")]),
        "a later attempt must see the painted cards"
    );
    recognition.shutdown();
}

/// Arming twice with the same relics must not start a second reader: squad
/// relics are logged once per member, so the same baseline can fire several
/// times within milliseconds.
#[test]
fn duplicate_baselines_reuse_the_single_capture_source() {
    let coordination = coordination(vec![
        Frame::Gated(Box::new(Frame::Names(names(&[
            entry("Perigale Prime Receiver"),
            entry("Vasto Prime Stock"),
        ])))),
        Frame::Names(names(&[
            entry("Perigale Prime Receiver"),
            entry("Vasto Prime Stock"),
        ])),
    ]);
    let mut guard = GateGuard(Some(coordination.stack_gate()));

    let mut recognition = recognition(&coordination);
    recognition.observe(&baseline(&[EARLY_RELIC]));
    wait_for_capture(&coordination, 1);
    // A second `BaselineRequested` with the same relics arrives mid-capture.
    recognition.observe(&baseline(&[EARLY_RELIC]));
    guard.release();
    // The single source finishes its read; shutdown joins it.
    recognition.shutdown();
    assert_eq!(
        coordination.made.load(Ordering::Acquire),
        1,
        "duplicate baselines must not create a second capture source"
    );
}

/// An empty pool means the closed-set match has nothing to match against, so
/// nothing captures; the later non-empty baseline starts the reader.
#[test]
fn an_empty_pool_does_not_capture_until_a_baseline_populates_it() {
    let coordination = coordination(vec![Frame::Names(names(&[
        entry("Perigale Prime Receiver"),
        entry("Vasto Prime Stock"),
    ]))]);
    let mut recognition = recognition(&coordination);
    recognition.observe(&baseline(&[]));
    assert!(
        coordination.captures.load(Ordering::Acquire) == 0,
        "an empty pool must not capture"
    );
    std::thread::sleep(Duration::from_millis(80)); // several cadence intervals
    assert!(
        coordination.captures.load(Ordering::Acquire) == 0,
        "an empty pool must not capture across cadence intervals"
    );
    assert!(
        recognition.candidates().is_empty(),
        "no relics in the baseline resolves to no candidates"
    );

    recognition.observe(&baseline(&[EARLY_RELIC]));
    let update = drain_recognized(&mut recognition);
    assert_eq!(
        update.names,
        names(&[entry("Perigale Prime Receiver"), entry("Vasto Prime Stock")])
    );
    recognition.shutdown();
}

/// The 2026-07-27 bug, ported: squad relics load one at a time, a baseline can
/// fire on the second of four, and the two later relics must still reach the
/// running reader as the pool grows.
#[test]
fn a_relic_that_loads_after_the_baseline_still_grows_the_pool() {
    let coordination = coordination(vec![Frame::Names(names(&[
        entry("Perigale Prime Receiver"),
        entry("Vasto Prime Stock"),
    ]))]);
    let mut recognition = recognition(&coordination);
    recognition.observe(&baseline(&[EARLY_RELIC]));
    let first = drain_recognized(&mut recognition);
    assert_eq!(
        first.names,
        names(&[entry("Perigale Prime Receiver"), entry("Vasto Prime Stock")])
    );

    // The third squad member's relic finishes loading after the reader is
    // already running; the pool the running reader matches against must grow.
    recognition.observe(&baseline(&[EARLY_RELIC, GROWN_RELIC]));
    let mut grown = names(&recognition.pool_entries());
    grown.sort();
    let mut expected = names(&[
        entry("Perigale Prime Receiver"),
        entry("Vasto Prime Stock"),
        entry("Burston Prime Receiver"),
        entry("Forma Blueprint"),
    ]);
    expected.sort();
    assert_eq!(
        grown, expected,
        "a relic that loads after the baseline must reach the running reader"
    );
    recognition.shutdown();
}

/// The 2026-08-20 bug, ported: a later fissure must replace the earlier one's
/// pool even when its own pool is smaller, or the closed-set match fabricates
/// confident nonsense instead of saying "not in the pool".
#[test]
fn a_later_fissure_replaces_an_earlier_larger_pool() {
    let coordination = coordination(vec![Frame::Names(names(&[
        entry("Perigale Prime Receiver"),
        entry("Vasto Prime Stock"),
    ]))]);
    let mut recognition = recognition(&coordination);
    recognition.observe(&baseline(&[EARLY_RELIC]));
    let _ = drain_recognized(&mut recognition);

    // The later fissure's single relic resolves to two names; the earlier one
    // resolved to three. The later, smaller pool must replace the larger one.
    recognition.observe(&baseline(&[LATER_RELIC]));
    assert_eq!(
        names(&recognition.pool_entries()),
        names(&[
            entry("Burston Prime Receiver"),
            entry("Xaku Prime Neuroptics Blueprint"),
        ]),
        "the earlier fissure's rewards must not survive into the later one"
    );
    recognition.shutdown();
}

/// A baseline that replaces the pool while a capture is in flight must never
/// let the stale evidence publish against the new pool: the read was taken
/// under the *old* pool, so it can only be evidence for that pool. Both pool
/// replacements and constraint changes bump the evidence revision, retiring
/// the in-flight read.
#[test]
fn a_baseline_swap_mid_capture_never_publishes_the_stale_read() {
    let coordination = coordination(vec![
        Frame::Gated(Box::new(Frame::Names(names(&[
            entry("Perigale Prime Receiver"),
            entry("Vasto Prime Stock"),
        ])))),
        // A second gate holds the worker again after the stale delivery, so the
        // test sees the wire out of the old screen before the worker can cycle.
        Frame::Gated(Box::new(Frame::Blank)),
    ]);
    let mut guard_swap = GateGuard(Some(coordination.stack_gate()));
    let mut guard_hold = GateGuard(Some(coordination.stack_gate()));

    let mut recognition = recognition(&coordination);
    // The later fissure opens while the earlier one's capture is still in flight.
    recognition.observe(&baseline(&[EARLY_RELIC]));
    wait_for_capture(&coordination, 1);
    recognition.observe(&baseline(&[LATER_RELIC]));
    guard_swap.release();
    // The worker is now wedged on the second gate: nothing else can happen
    // until released, so a stale publish can only arrive from the old screen.
    wait_for_capture(&coordination, 2);
    guard_hold.release();
    drain_and_assert_nothing_recognized(&mut recognition);
    recognition.shutdown();
}

/// A read that is missing the logged local reward, or whose card count does
/// not match, is not published -- the mismatched set must be seen rejected
/// before the valid one publishes.
#[test]
fn known_constraints_reject_mismatched_reads() {
    for (first, second) in [
        // Missing the logged local reward.
        (
            Frame::Names(names(&[
                entry("Vasto Prime Stock"),
                entry("Burston Prime Receiver"),
            ])),
            Frame::Names(names(&[
                entry("Perigale Prime Receiver"),
                entry("Vasto Prime Stock"),
            ])),
        ),
        // Rendered count 1 vs the logged count 2.
        (
            Frame::Names(names(&[entry("Perigale Prime Receiver")])),
            Frame::Names(names(&[
                entry("Perigale Prime Receiver"),
                entry("Vasto Prime Stock"),
            ])),
        ),
    ] {
        let coordination = coordination(vec![first, second]);
        let mut recognition = recognition(&coordination);
        recognition.observe(&baseline(&[EARLY_RELIC]));
        recognition.observe(&choices_ready(
            2,
            Some("/Lotus/StoreItems/Types/Recipes/Weapons/PerigalePrimeReceiverComponent"),
        ));
        let deadline = Instant::now() + Duration::from_secs(5);
        // The mismatch must be withheld: it may surface as an explicit failure update or be
        // suppressed inside the fast retry window, but it must never publish. Waiting for at
        // least one non-accepted poll before the valid read proves the first frame did not
        // publish; the name assertion below proves what published was the valid frame.
        let mut saw_withheld = false;
        loop {
            let update = recognition.poll();
            if let Some(recognized) = update.recognized {
                assert_eq!(
                    recognized.names,
                    names(&[entry("Perigale Prime Receiver"), entry("Vasto Prime Stock")]),
                    "unexpected accepted read"
                );
                break;
            }
            saw_withheld = true;
            if Instant::now() >= deadline {
                panic!("no valid read published after the rejection");
            }
            std::thread::yield_now();
        }
        assert!(saw_withheld, "the mismatched read must be observed");
    }
}

/// `ChoicesReady`'s card count outranks the `ResponsesComplete` roster size:
/// a read matching the roster but not the rendered count must not publish.
#[test]
fn choices_ready_count_outranks_the_roster_size() {
    let coordination = coordination(vec![
        Frame::Names(names(&[
            entry("Perigale Prime Receiver"),
            entry("Vasto Prime Stock"),
            entry("Burston Prime Receiver"),
            entry("Forma Blueprint"),
        ])),
        Frame::Names(names(&[
            entry("Perigale Prime Receiver"),
            entry("Vasto Prime Stock"),
        ])),
    ]);
    let mut recognition = recognition(&coordination);
    recognition.observe(&baseline(&[EARLY_RELIC]));
    recognition.observe(&RewardLogEvent::ResponsesComplete {
        responders: vec![
            "de1e7ed00000000000000005".into(),
            "de1e7ed0000000000000000a".into(),
            "de1e7ed0000000000000000f".into(),
            "de1e7ed00000000000000010".into(),
        ],
        screen_order: vec![
            "de1e7ed00000000000000005".into(),
            "de1e7ed0000000000000000a".into(),
            "de1e7ed0000000000000000f".into(),
            "de1e7ed00000000000000010".into(),
        ],
        local_reward_path: None,
        local_identity: None,
    });
    recognition.observe(&choices_ready(2, None));
    let update = drain_recognized(&mut recognition);
    assert_eq!(
        update.names,
        names(&[entry("Perigale Prime Receiver"), entry("Vasto Prime Stock")]),
        "the roster's size must not outrank ChoicesReady's rendered count"
    );
    recognition.shutdown();
}

/// A screen that has already gone stops the reader instead of grinding retries
/// on the monitor thread: close is prompt and no later result is delivered.
#[test]
fn close_is_prompt_while_a_capture_is_still_in_flight() {
    let coordination = coordination(vec![Frame::Gated(Box::new(Frame::Names(names(&[
        entry("Perigale Prime Receiver"),
        entry("Vasto Prime Stock"),
    ]))))]);
    let mut guard = GateGuard(Some(coordination.stack_gate()));

    let mut recognition = recognition(&coordination);
    recognition.observe(&baseline(&[EARLY_RELIC]));
    wait_for_capture(&coordination, 1);

    let started = Instant::now();
    recognition.observe(&RewardLogEvent::Closed); // must not join or wait on the worker
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "observe(Closed) blocked on the in-flight capture"
    );
    guard.release();
    drain_and_assert_nothing_recognized(&mut recognition);
    recognition.shutdown();
}

/// A transient process loss must not blind the fissure: suspend retires in-flight work and
/// pauses capture without clearing the pool, and resume publishes against the preserved pool.
/// Baselines are one-shot log lines, so clearing on every absence would be unrecoverable.
#[test]
fn suspend_preserves_the_pool_across_a_transient_process_loss() {
    let coordination = coordination(vec![Frame::Names(names(&[
        entry("Perigale Prime Receiver"),
        entry("Vasto Prime Stock"),
    ]))]);
    let mut recognition = recognition(&coordination);
    recognition.observe(&baseline(&[EARLY_RELIC]));
    // A discovery blip while no screen is up: nothing captured yet, pool preserved.
    recognition.suspend();
    recognition.suspend(); // idempotent: repeated absence ticks do no further work
    assert_eq!(recognition.pool_entries().len(), 2);
    recognition.resume();
    let update = drain_recognized(&mut recognition);
    assert_eq!(
        update.names,
        names(&[entry("Perigale Prime Receiver"), entry("Vasto Prime Stock")])
    );
    recognition.shutdown();
}

/// The fifth-review bugs: at cold start (no session, epoch 0) suspend must not bump the
/// epoch, or it would carry resolved-epoch 0 to a live value -- suppressing the first
/// fissure's reads as duplicates -- and arm a 45-minute watch for a session that never
/// opened, parking recognition at its expiry. Suspend before any baseline, then verify the
/// very first screen publishes normally.
#[test]
fn suspend_at_cold_start_does_not_suppress_the_first_screen() {
    let coordination = coordination(vec![Frame::Names(names(&[
        entry("Perigale Prime Receiver"),
        entry("Vasto Prime Stock"),
    ]))]);
    let mut recognition = recognition(&coordination);
    // Game absent at launch: monitor suspends before any baseline opened a session.
    recognition.suspend();
    recognition.suspend(); // idempotent
    recognition.resume();
    // The first fissure's baseline opens epoch 1 for the first time.
    recognition.observe(&baseline(&[EARLY_RELIC]));
    let update = drain_recognized(&mut recognition);
    assert_eq!(
        update.names,
        names(&[entry("Perigale Prime Receiver"), entry("Vasto Prime Stock")]),
        "the first screen after a cold-start suspend must publish, not be dropped as a duplicate"
    );
    recognition.shutdown();
}

/// The close-discard regression: a result that is in flight (or queued) when
/// the screen closes must never be delivered to the consumer.
#[test]
fn close_discards_an_undelivered_reward_read() {
    let coordination = coordination(vec![Frame::Gated(Box::new(Frame::Names(names(&[
        entry("Perigale Prime Receiver"),
        entry("Vasto Prime Stock"),
    ]))))]);
    let mut guard = GateGuard(Some(coordination.stack_gate()));

    let mut recognition = recognition(&coordination);
    recognition.observe(&baseline(&[EARLY_RELIC]));
    wait_for_capture(&coordination, 1);
    // The screen closes while the capture is still in flight.
    recognition.close();
    guard.release();
    drain_and_assert_nothing_recognized(&mut recognition);
    recognition.shutdown();
}

/// One blank read mid-screen must not close the overlay; two consecutive
/// misses after the screen was found must. Both misses are gated so the test
/// observes each delivery in order instead of racing the worker's cadence.
#[test]
fn two_misses_close_the_screen_without_a_one_blank_flicker() {
    let coordination = coordination(vec![
        Frame::Names(names(&[
            entry("Perigale Prime Receiver"),
            entry("Vasto Prime Stock"),
        ])),
        Frame::Gated(Box::new(Frame::Blank)),
        Frame::Gated(Box::new(Frame::Blank)),
    ]);
    let mut first_miss = GateGuard(Some(coordination.stack_gate()));
    let mut second_miss = GateGuard(Some(coordination.stack_gate()));
    let mut recognition = recognition(&coordination);
    recognition.observe(&baseline(&[EARLY_RELIC]));
    let update = drain_recognized(&mut recognition);
    assert_eq!(
        update.names,
        names(&[entry("Perigale Prime Receiver"), entry("Vasto Prime Stock")])
    );

    // First miss: a failure update, but the overlay must not hide. The worker is
    // parked on the second gate afterwards, so the failure stays observable.
    first_miss.release();
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut saw_first_miss = false;
    while Instant::now() < deadline {
        let update = recognition.poll();
        assert!(!update.hide, "one blank read must not close the overlay");
        if update.failure.is_some() {
            saw_first_miss = true;
            break;
        }
        std::thread::yield_now();
    }
    assert!(saw_first_miss, "the first blank read was never reported");

    // Second miss: the screen is gone.
    second_miss.release();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let update = recognition.poll();
        if update.hide {
            break;
        }
        if Instant::now() >= deadline {
            panic!("two misses never hid the screen");
        }
        std::thread::yield_now();
    }
    recognition.shutdown();
}

/// `shutdown` joins the worker and waits for the capture-thread source to be
/// destroyed, even when a capture is in flight when it is requested.
#[test]
fn shutdown_joins_the_in_flight_capture_destruction() {
    let coordination = coordination(vec![Frame::Gated(Box::new(Frame::Names(names(&[
        entry("Perigale Prime Receiver"),
        entry("Vasto Prime Stock"),
    ]))))]);
    let mut guard = GateGuard(Some(coordination.stack_gate()));
    let (dropped_tx, dropped_rx) = mpsc::channel();
    *coordination.dropped.lock().expect("drop lock") = dropped_tx;

    let mut recognition = recognition(&coordination);
    recognition.observe(&baseline(&[EARLY_RELIC]));
    wait_for_capture(&coordination, 1);

    let (shutdown_started_tx, shutdown_started_rx) = mpsc::channel();
    let (shutdown_done_tx, shutdown_done_rx) = mpsc::channel();
    let mut recognition_on_thread = recognition;
    let shutdown_thread = std::thread::spawn(move || {
        shutdown_started_tx
            .send(())
            .expect("test thread is listening");
        recognition_on_thread.shutdown();
        shutdown_done_tx.send(()).expect("test thread is listening");
    });
    shutdown_started_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("shutdown thread started");
    guard.release();
    shutdown_done_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("shutdown joined the worker");
    shutdown_thread.join().expect("shutdown thread joined");

    // The drop happened on the worker thread before shutdown returned: the
    // source's destructor signalled within the join window.
    assert!(
        dropped_rx.recv_timeout(Duration::from_secs(1)).is_ok(),
        "shutdown must wait for the in-flight source to be destroyed"
    );
}

/// A publication token from the current screen publishes exactly once; the
/// same token declines a duplicate effect, and a token from a retired screen
/// always declines -- current versus retired publication tokens.
#[test]
fn publication_gate_accepts_the_current_token_and_declines_retired_duplicates() {
    let coordination = coordination(vec![
        Frame::Names(names(&[
            entry("Perigale Prime Receiver"),
            entry("Vasto Prime Stock"),
        ])),
        Frame::Names(names(&[
            entry("Burston Prime Receiver"),
            entry("Xaku Prime Neuroptics Blueprint"),
        ])),
    ]);
    let mut recognition = recognition(&coordination);
    recognition.observe(&baseline(&[EARLY_RELIC]));
    let current = drain_recognized(&mut recognition);

    let first_effect = Arc::new(AtomicUsize::new(0));
    assert!(
        current.publication.publish(|| {
            first_effect.fetch_add(1, Ordering::AcqRel);
        }),
        "the current token must publish"
    );
    assert_eq!(first_effect.load(Ordering::Acquire), 1);
    assert!(
        !current.publication.publish(|| {
            first_effect.fetch_add(1, Ordering::AcqRel);
        }),
        "the same publication must not run a second effect"
    );
    assert_eq!(
        first_effect.load(Ordering::Acquire),
        1,
        "duplicate effect ran"
    );

    // Closing the screen and opening the next fissure retires the old
    // token: its effect must decline even though the caller still holds it.
    recognition.close();
    recognition.observe(&baseline(&[LATER_RELIC]));
    let next = drain_recognized(&mut recognition);
    let retired_effect = Arc::new(AtomicUsize::new(0));
    assert!(
        !current.publication.publish(|| {
            retired_effect.fetch_add(1, Ordering::AcqRel);
        }),
        "a token from a retired screen must decline"
    );
    assert_eq!(retired_effect.load(Ordering::Acquire), 0);
    assert!(
        next.publication.publish(|| {
            retired_effect.fetch_add(1, Ordering::AcqRel);
        }),
        "the new screen's token must publish"
    );
    assert_eq!(retired_effect.load(Ordering::Acquire), 1);
    recognition.shutdown();
}

/// A published read records the pool it was matched against at a level a
/// stable build keeps (Info or below), naming the relics and the pool size.
/// The assertion is built on the public interface (`pool_entries`) and only
/// checks for the presence of a concise provenance line, never raw wording.
#[test]
fn a_recognized_read_logs_its_pool_provenance() {
    let coordination = coordination(vec![Frame::Names(names(&[
        entry("Perigale Prime Receiver"),
        entry("Vasto Prime Stock"),
    ]))]);
    let mut recognition = recognition(&coordination);
    let provenance = capture_log(|| {
        recognition.observe(&baseline(&[EARLY_RELIC]));
        let _ = drain_recognized(&mut recognition);
        names(&recognition.pool_entries());
    });
    // Other tests in this binary publish concurrently with the same card names; disambiguate
    // by this test's relic and pool size rather than taking the first provenance line.
    let pool_line = provenance.iter().find(|(_, line)| {
        line.contains("Perigale Prime Receiver")
            && line.contains("pool=2")
            && line.contains("T1VoidProjectionEarlyBronze")
    });
    let (level, line) = pool_line.expect("a published read must log its pool provenance");
    assert!(
        *level <= log::Level::Info,
        "pool provenance must survive a stable build's file filter: {line}"
    );
    recognition.shutdown();
}

// Pool provenance logging recorder: replaces the binary-wide logger once so
// tests in this file can name the level and the pool size without asserting
// wording. Tests outside this file that log at Info or below are unaffected
// in behaviour; the old `RelicPool` recorder did the same.
static LINES: Mutex<Vec<(log::Level, String)>> = Mutex::new(Vec::new());
static INSTALL: std::sync::Once = std::sync::Once::new();
static SERIAL: Mutex<()> = Mutex::new(());

struct Capture;

impl log::Log for Capture {
    fn enabled(&self, _: &log::Metadata<'_>) -> bool {
        true
    }
    fn log(&self, record: &log::Record<'_>) {
        LINES
            .lock()
            .expect("lines lock")
            .push((record.level(), record.args().to_string()));
    }
    fn flush(&self) {}
}

fn capture_log(emit: impl FnOnce()) -> Vec<(log::Level, String)> {
    let _serial = SERIAL.lock().expect("serial lock");
    INSTALL.call_once(|| {
        log::set_boxed_logger(Box::new(Capture)).expect("logger installs once");
        log::set_max_level(log::LevelFilter::Debug);
    });
    LINES.lock().expect("lines lock").clear();
    emit();
    LINES.lock().expect("lines lock").clone()
}
