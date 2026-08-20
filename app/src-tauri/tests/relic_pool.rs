//! Which fissure's relics the poller matches a card against.
//!
//! The closed-set match is what makes a garbled read trustworthy: a card only has to land on the
//! nearest of two dozen known rewards, so a chipped letter still lands on the right item. That
//! argument holds exactly as long as the set is *this* fissure's. Against another fissure's names
//! the same match is not a safety net but a fabricator -- it cannot return "not in the pool", only
//! the nearest thing it was given, and at these string lengths the nearest thing scores 0.8 and
//! sails past the 0.6 floor.
//!
//! That is the 2026-08-20 report. Every card was wrong, every card was confidently wrong, and
//! nothing in the log said so.

use app_lib::RelicPool;
use warframe_acquisition::RewardCatalogEntry;

use std::sync::{Mutex, Once};

/// Captures what the pool logs, so the diagnostic can be asserted rather than eyeballed.
struct Capture;

static LINES: Mutex<Vec<(log::Level, String)>> = Mutex::new(Vec::new());
static INSTALL: Once = Once::new();
static SERIAL: Mutex<()> = Mutex::new(());

impl log::Log for Capture {
    fn enabled(&self, _metadata: &log::Metadata<'_>) -> bool {
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

fn captured(emit: impl FnOnce()) -> (log::Level, String) {
    INSTALL.call_once(|| {
        log::set_boxed_logger(Box::new(Capture)).expect("logger installs once");
        log::set_max_level(log::LevelFilter::Debug);
    });
    LINES.lock().expect("lines lock").clear();
    emit();
    LINES
        .lock()
        .expect("lines lock")
        .first()
        .cloned()
        .expect("nothing was logged")
}

fn entries<I, S>(names: I) -> Vec<RewardCatalogEntry>
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    names
        .into_iter()
        .map(|name| RewardCatalogEntry {
            name: name.into(),
            ducats: 15,
        })
        .collect()
}

/// The 17:46 fissure's relics. The log records the pool it grew to (`arm pool=38`) but not the
/// paths, so these stand for four relics of that session rather than quoting them.
fn earlier_relics() -> Vec<String> {
    (0..4)
        .map(|relic| format!("/Lotus/Types/Game/Projections/T3VoidProjectionEarlier{relic}Bronze"))
        .collect()
}

/// The 17:46 fissure's pool, at the 38 names `arm pool=38` records.
///
/// The first five are the names that session's own OCR traces show; the last two are the ones the
/// 19:19 misread proves were in it, because nothing else on that screen could have produced them.
/// The remainder stands in for names the log never spells out -- only the size relation matters to
/// the rule under test, and inventing plausible Warframe items would read as evidence it is not.
fn earlier_pool() -> Vec<RewardCatalogEntry> {
    const RECORDED: [&str; 7] = [
        "Xaku Prime Neuroptics Blueprint",
        "Okina Prime Blade",
        "Braton Prime Receiver",
        "Paris Prime Grip",
        "2X Forma Blueprint",
        "Yareli Prime Chassis Blueprint",
        "Xaku Prime Blueprint",
    ];
    entries(
        RECORDED
            .into_iter()
            .map(str::to_owned)
            .chain((RECORDED.len()..38).map(|name| format!("Unrecorded Pool Name {name}"))),
    )
}

/// The three relics EE.log names for the 19:19 fissure, and the only three it names.
fn later_relics() -> Vec<String> {
    [
        "/Lotus/Types/Game/Projections/T1VoidProjectionGyrePrimeDBronze",
        "/Lotus/Types/Game/Projections/T1VoidProjectionVorunaPrimeBPlatinum",
        "/Lotus/Types/Game/Projections/T1VoidProjectionStyanaxPrimeDBronze",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect()
}

/// Their union, read out of the cached WFCD relic index: Lith G14 Intact, Lith N19 Radiant and
/// Lith T14 Intact, sixteen names between them. All four rewards that were actually on screen are
/// here, which is the point -- the read that went wrong had nothing wrong with its pixels.
fn later_pool() -> Vec<RewardCatalogEntry> {
    entries([
        "2X Forma Blueprint",
        "Daikyu Prime Blueprint",
        "Daikyu Prime String",
        "Fang Prime Handle",
        "Forma Blueprint",
        "Gyre Prime Neuroptics Blueprint",
        "Lavos Prime Chassis Blueprint",
        "Nautilus Prime Cerebrum",
        "Paris Prime Lower Limb",
        "Paris Prime Upper Limb",
        "Perigale Prime Receiver",
        "Quassus Prime Blueprint",
        "Trumna Prime Barrel",
        "Vadarya Prime Receiver",
        "Vadarya Prime Stock",
        "Voruna Prime Chassis Blueprint",
    ])
}

fn holds(pool: &RelicPool, name: &str) -> bool {
    pool.entries().iter().any(|entry| entry.name == name)
}

/// The 2026-08-20 bug: a fissure inherited an earlier one's pool because that pool was bigger.
///
/// Two hours and an application restart separated them. The 19:19 squad's three relics resolve to
/// sixteen names, the 17:46 squad's four resolved to thirty-eight, and the rule for installing a
/// pool was "only if it is longer" -- so sixteen never displaced thirty-eight and every card on
/// the later screen was matched against relics nobody in that squad was carrying.
///
/// What reached the overlay: `Forma Blueprint` read as `2X Forma Blueprint`, `Lavos Prime Chassis
/// Blueprint` as `Yareli Prime Chassis Blueprint`, `Daikyu Prime Blueprint` as `Xaku Prime
/// Blueprint` -- 0.88, 0.82 and 0.85 against a 0.60 floor. Nothing failed, so nothing was logged.
#[test]
fn a_later_fissure_replaces_an_earlier_larger_pool() {
    let mut pool = RelicPool::default();
    pool.adopt(&earlier_relics(), earlier_pool());
    pool.adopt(&later_relics(), later_pool());

    assert!(
        !holds(&pool, "Xaku Prime Blueprint"),
        "the earlier fissure's names survived into a later one: `Daikyu Prime Blueprint` was on \
         screen and matched `Xaku Prime Blueprint` at 0.85 because this name was still here"
    );
    assert!(
        !holds(&pool, "Yareli Prime Chassis Blueprint"),
        "`Lavos Prime Chassis Blueprint` was on screen and matched this at 0.82"
    );
    assert!(
        holds(&pool, "Daikyu Prime Blueprint") && holds(&pool, "Lavos Prime Chassis Blueprint"),
        "the squad's own rewards have to be in the pool for the closed-set match to find them"
    );
}

/// The 2026-07-27 bug, which is why the length rule existed and why the fix must not simply drop
/// pool updates it thinks are redundant.
///
/// Squad relics are logged one at a time and the baseline fires on the second of four, so a pool
/// is *supposed* to grow mid-fissure -- the poller re-reads it every poll for exactly that reason.
/// A rule keyed on the relic list has to keep letting that through: these relics extend the ones
/// already adopted, so this is the same fissure learning about a squadmate, not a new one.
#[test]
fn a_relic_that_loads_later_in_the_same_fissure_still_grows_the_pool() {
    let mut pool = RelicPool::default();
    let (first_two, all_three) = (&later_relics()[..2], later_relics());
    pool.adopt(first_two, entries(["Gyre Prime Neuroptics Blueprint"]));
    pool.adopt(&all_three, later_pool());

    assert!(
        holds(&pool, "Trumna Prime Barrel"),
        "the third squad member's relic never reached the pool; one unmatched card fails the \
         whole read, so the overlay would never appear"
    );
    assert_eq!(pool.len(), later_pool().len());
}

/// The 2026-08-20 report contained no evidence of what went wrong.
///
/// Every card was published above the match floor, so nothing failed and nothing was logged. The
/// one line that would have settled it -- which relics the pool came from -- exists only as
/// `[DEBUG-poller] arm pool=38`, at Debug, which the stable build's file target filters out. The
/// report shipped a wall of unrelated warnings and the mismatch had to be reconstructed afterwards
/// by reading the squad's relics out of the cached catalog by hand.
///
/// So a published read says what it was matched against, at a level a stable build keeps.
#[test]
fn a_published_read_records_the_relics_it_was_matched_against() {
    let _serial = SERIAL.lock().expect("serial lock");
    let mut pool = RelicPool::default();
    pool.adopt(&later_relics(), later_pool());

    let (level, line) = captured(|| pool.trace_published(&["Forma Blueprint".to_owned()]));

    assert!(
        level <= log::Level::Info,
        "logged at {level}, which the stable build's `<= Info` file filter drops -- the level is \
         the whole point of the line"
    );
    assert!(
        line.contains("T1VoidProjectionGyrePrimeDBronze"),
        "the line has to name the relics or it cannot say which fissure the pool belongs to: {line}"
    );
    assert!(
        line.contains("pool=16"),
        "the line has to size the pool, which is what made the stale one recognisable: {line}"
    );
    assert!(
        line.contains("Forma Blueprint"),
        "the line has to say what was published, or it cannot be tied to the overlay: {line}"
    );
}
