use serde::{Deserialize, Serialize};

/// What the collection says about the item a reward builds. A mark the evidence cannot support
/// is `Unknown`, drawn as a dash; items mastery never touches carry no mark.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum MasteryMark {
    Mastered,
    Built {
        rank: u32,
        max_rank: u32,
    },
    InFoundry,
    Unmastered {
        subject: Option<String>,
        parts: Vec<SetPart>,
        missing: bool,
        completes: bool,
    },
    Unknown,
}

/// One slot of the set behind an unmastered reward: what the slot needs, what the player
/// holds, and whether this reward fills it.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SetPart {
    pub name: String,
    pub image: Option<String>,
    pub uses: u32,
    pub held: u32,
    /// The slot this reward fills.
    pub this: bool,
}

/// A part of an item the player has not mastered: how many copies they hold, and how many
/// one build uses. Kiosk tiles name parts, so this is all a strip has room to say.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct KioskMastery {
    pub held: u32,
    pub uses: u32,
}
