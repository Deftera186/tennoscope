//! Scores every labelled fixture card. Not a test: it is how the resize filter in `prepare_crop`
//! was chosen, kept so the choice can be re-swept rather than trusted.
fn main() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let relic = [
        "Braton Prime Blueprint",
        "2X Forma Blueprint",
        "Burston Prime Stock",
        "Trumna Prime Blueprint",
        "Braton Prime Receiver",
        "Burston Prime Receiver",
        "Trumna Prime Barrel",
        "Paris Prime Lower Limb",
    ];
    let wrapped = [
        "Caliban Prime Chassis Blueprint",
        "Caliban Prime Blueprint",
        "Caliban Prime Neuroptics Blueprint",
        "Bronco Prime Receiver",
        "Bronco Prime Barrel",
        "Forma Blueprint",
    ];
    for (fixture, pool) in [
        ("reward-screen-1920x1080.png", &relic[..]),
        ("reward-screen-three-cards.png", &relic[..]),
        ("reward-screen-wrapped-title.png", &wrapped[..]),
    ] {
        let pool: Vec<_> = pool
            .iter()
            .map(|name| warframe_acquisition::RewardCatalogEntry {
                name: (*name).to_owned(),
                ducats: 0,
            })
            .collect();
        match app_lib::read_cards(&dir.join(fixture), &pool) {
            Ok(cards) => {
                for (name, score) in cards {
                    println!("  {name:36} {score:.4}");
                }
            }
            Err(reason) => println!("  FAILED: {reason}"),
        }
    }
}
