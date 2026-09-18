use serde::{Deserialize, Serialize};

/// The maximum Warframe observation the player has enabled.
///
/// Ordering is intentional: Settings can distinguish access-removing transitions from access-
/// adding ones without maintaining a second interpretation of the modes.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum AccessMode {
    Companion,
    Overlay,
    Full,
}

impl AccessMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Companion => "companion",
            Self::Overlay => "overlay",
            Self::Full => "full",
        }
    }

    pub const fn policy(self) -> AccessPolicy {
        match self {
            Self::Companion => AccessPolicy::COMPANION,
            Self::Overlay => AccessPolicy::OVERLAY,
            Self::Full => AccessPolicy::FULL,
        }
    }
}

/// Backend authorization for every capability that observes the running game.
///
/// Desktop catalog, saved collection, pricing, and optional warframe.market features are absent
/// because no access mode restricts them.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AccessPolicy {
    pub observe_process_presence: bool,
    pub observe_ee_log: bool,
    pub capture_screen: bool,
    pub show_overlays: bool,
    pub read_process_memory: bool,
    pub acquire_inventory: bool,
}

impl AccessPolicy {
    pub const COMPANION: Self = Self {
        observe_process_presence: false,
        observe_ee_log: false,
        capture_screen: false,
        show_overlays: false,
        read_process_memory: false,
        acquire_inventory: false,
    };
    const OVERLAY: Self = Self {
        observe_process_presence: true,
        observe_ee_log: true,
        capture_screen: true,
        show_overlays: true,
        read_process_memory: false,
        acquire_inventory: false,
    };
    const FULL: Self = Self {
        read_process_memory: true,
        acquire_inventory: true,
        ..Self::OVERLAY
    };
}

impl AccessPolicy {
    pub const fn starts_monitor(self) -> bool {
        self.observe_process_presence
    }

    pub const fn uses_game_memory(self) -> bool {
        self.read_process_memory || self.acquire_inventory
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn policies_are_the_single_mode_boundary() {
        assert!(!AccessMode::Companion.policy().starts_monitor());
        assert!(AccessMode::Overlay.policy().starts_monitor());
        assert!(AccessMode::Full.policy().starts_monitor());
        assert!(!AccessMode::Overlay.policy().uses_game_memory());
        assert!(AccessMode::Full.policy().uses_game_memory());
    }
}
