use std::collections::{BTreeMap, BTreeSet};

use warframe_domain::{InventoryEntry, KioskMastery, MasteryMark, SetPart};

use crate::{CatalogIndex, ComponentKind, MasteryFacts, Recipe, RecipeComponent, mastery_rank};

/// Whether the marks may speak from this run's inventory, from a saved collection, or not
/// at all.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MasteryEvidence {
    /// Facts from a successful inventory sync in this run: a mark may claim any state.
    Live,
    /// A saved collection but no facts from this run: a mark says Mastered or Unknown.
    Saved,
    /// No collection at all: every part of a masterable item reads Unknown.
    None,
}

/// What the player holds, keyed by catalog path: quantities summed across rank variants,
/// and the items the game marks mastered.
#[derive(Clone, Debug, Default)]
pub struct Holdings {
    quantity: BTreeMap<String, u32>,
    mastered: BTreeSet<String>,
}

impl Holdings {
    /// Sums the rank variants of one catalog path (`<path>#<rank>`) into one quantity. A mastered
    /// entry at zero quantity, an item since sold, still counts as mastered.
    pub fn from_entries<'a>(entries: impl IntoIterator<Item = &'a InventoryEntry>) -> Self {
        let mut holdings = Self::default();
        for entry in entries {
            let path = entry.item.id.catalog_path();
            *holdings.quantity.entry(path.to_owned()).or_default() += entry.quantity;
            if entry.mastered {
                holdings.mastered.insert(path.to_owned());
            }
        }
        holdings
    }

    /// Maps already keyed by catalog path, summed by the caller. A mastered path needs no quantity.
    pub fn from_parts(quantity: BTreeMap<String, u32>, mastered: BTreeSet<String>) -> Self {
        Self { quantity, mastered }
    }

    /// Copies at exactly this path, zero when none: a part and its blueprint sibling count apart.
    pub fn quantity(&self, path: &str) -> u32 {
        self.quantity.get(path).copied().unwrap_or(0)
    }

    /// True once the game marks the item mastered, and still true after the item is sold.
    pub fn mastered(&self, path: &str) -> bool {
        self.mastered.contains(path)
    }

    /// Nothing held and nothing mastered. Without live facts, this is what makes evidence None.
    pub fn is_empty(&self) -> bool {
        self.quantity.is_empty() && self.mastered.is_empty()
    }
}

/// Turns a catalog, a collection and this run's facts into the mark each reward carries. A
/// saved collection proves mastery but never a missing part, so kiosk strips need live facts.
pub struct MasteryLedger<'a> {
    catalog: &'a CatalogIndex,
    holdings: &'a Holdings,
    facts: Option<&'a MasteryFacts>,
}

impl<'a> MasteryLedger<'a> {
    /// `holdings` may be a saved collection; `facts` must come from this run's inventory sync.
    pub fn new(
        catalog: &'a CatalogIndex,
        holdings: &'a Holdings,
        facts: Option<&'a MasteryFacts>,
    ) -> Self {
        Self {
            catalog,
            holdings,
            facts,
        }
    }

    /// Live whenever facts are present, even over an empty collection; Saved needs a non-empty one.
    pub fn evidence(&self) -> MasteryEvidence {
        if self.facts.is_some() {
            MasteryEvidence::Live
        } else if !self.holdings.is_empty() {
            MasteryEvidence::Saved
        } else {
            MasteryEvidence::None
        }
    }

    /// None unless the name is a part of a masterable item, so never for Forma or a Kavasa part.
    pub fn reward_mark(&self, reward_name: &str) -> Option<MasteryMark> {
        let (recipe, slot_index) = self.resolve(reward_name)?;
        let parent = recipe.parent.as_str();
        match self.evidence() {
            MasteryEvidence::None => Some(MasteryMark::Unknown),
            MasteryEvidence::Saved => {
                if self.closed(parent) && self.all_consumers_closed(parent) {
                    Some(MasteryMark::Mastered)
                } else {
                    Some(MasteryMark::Unknown)
                }
            }
            MasteryEvidence::Live => Some(self.live_mark(recipe, slot_index)),
        }
    }

    /// Counts for a kiosk strip, only under live facts and only while the part's own item is not
    /// mastered, built or in the foundry, so never for a chain mark.
    pub fn kiosk_mastery(&self, part_name: &str) -> Option<KioskMastery> {
        if self.evidence() != MasteryEvidence::Live {
            return None;
        }
        match self.reward_mark(part_name)? {
            MasteryMark::Unmastered {
                subject: None,
                parts,
                ..
            } => {
                let this = parts.iter().find(|slot| slot.this)?;
                Some(KioskMastery {
                    held: this.held,
                    uses: this.uses,
                })
            }
            _ => None,
        }
    }

    /// A reward name to its parent recipe and the slot it fills there. Nothing when the name
    /// is not a part, or when the parent is an item mastery never touches.
    fn resolve(&self, reward_name: &str) -> Option<(&'a Recipe, usize)> {
        let path = self.catalog.part_path_for_reward(reward_name)?;
        let (recipe, slot) = self.catalog.part_parent(path)?;
        if !self.catalog.resolve(&recipe.parent)?.masterable() {
            return None;
        }
        let slot_index = recipe
            .components
            .iter()
            .position(|candidate| candidate.path == slot.path)?;
        Some((recipe, slot_index))
    }

    fn live_mark(&self, recipe: &'a Recipe, slot_index: usize) -> MasteryMark {
        let parent = recipe.parent.as_str();
        let have = self.have(parent);
        if !self.closed(parent) && have == 0 {
            let multiplier = self.want(parent);
            return self.unmastered(recipe, slot_index, multiplier, None);
        }
        let open = self.open_consumers(parent).collect::<Vec<_>>();
        let need = open.iter().map(|(_, per)| *per).sum::<u32>();
        // Prefer a consumer short even on its own: its row is the one that shows a copy missing.
        let chain = open
            .iter()
            .find(|(_, per)| have < *per)
            .or(open.first())
            .filter(|_| have < need)
            .and_then(|(consumer, _)| {
                let recipe = self.catalog.recipe(consumer)?;
                let this_index = recipe.components.iter().position(|slot| {
                    slot.kind == ComponentKind::Ingredient && slot.path == parent
                })?;
                Some((recipe, this_index))
            });
        if let Some((consumer_recipe, this_index)) = chain {
            // The chain word names the consumer the way the design copy reads: no " Prime".
            let subject = consumer_recipe
                .parent_name
                .strip_suffix(" Prime")
                .unwrap_or(consumer_recipe.parent_name.as_str())
                .to_string();
            return self.unmastered(consumer_recipe, this_index, 1, Some(subject));
        }
        if !self.closed(parent) && self.built(parent) > 0 {
            let (rank, max_rank) = self
                .catalog
                .resolve(parent)
                .map(|meta| {
                    let rank = meta
                        .category()
                        .and_then(|category| {
                            mastery_rank(
                                category,
                                self.facts.map_or(0, |facts| facts.xp(parent)),
                                meta.max_rank(),
                            )
                        })
                        .unwrap_or(0);
                    (rank, meta.max_rank())
                })
                .unwrap_or((0, 0));
            return MasteryMark::Built { rank, max_rank };
        }
        if !self.closed(parent) && self.pending(parent) > 0 {
            return MasteryMark::InFoundry;
        }
        MasteryMark::Mastered
    }

    fn unmastered(
        &self,
        view: &Recipe,
        this_index: usize,
        multiplier: u32,
        subject: Option<String>,
    ) -> MasteryMark {
        let parts = view
            .components
            .iter()
            .enumerate()
            .map(|(index, slot)| SetPart {
                name: slot.name.clone(),
                image: slot.image_name.clone(),
                uses: slot.per_build.saturating_mul(multiplier),
                held: self.held_of(slot),
                this: index == this_index,
            })
            .collect::<Vec<_>>();
        let this_slot = &parts[this_index];
        let missing = this_slot.held < this_slot.uses;
        let completes = subject.is_none()
            && missing
            && this_slot.held.saturating_add(1) >= this_slot.uses
            && parts
                .iter()
                .enumerate()
                .all(|(index, slot)| index == this_index || slot.held >= slot.uses);
        MasteryMark::Unmastered {
            subject,
            parts,
            missing,
            completes,
        }
    }

    fn closed(&self, item: &str) -> bool {
        self.holdings.mastered(item)
    }

    fn built(&self, item: &str) -> u32 {
        self.holdings.quantity(item)
    }

    fn pending(&self, item: &str) -> u32 {
        match (self.facts, self.catalog.recipe(item)) {
            (Some(facts), Some(recipe)) => recipe
                .components
                .iter()
                .find(|slot| slot.kind == ComponentKind::Blueprint)
                .map_or(0, |blueprint| facts.pending(&blueprint.path)),
            _ => 0,
        }
    }

    fn have(&self, item: &str) -> u32 {
        self.built(item).saturating_add(self.pending(item))
    }

    /// How many builds still want one more copy: the larger of mastering one and feeding
    /// every open consumer, never the sum. One copy can be mastered and then consumed.
    fn want(&self, item: &str) -> u32 {
        let want_self = if self.closed(item) || self.have(item) > 0 {
            0
        } else {
            1
        };
        let want_chain = self
            .open_consumers(item)
            .map(|(_, per_build)| *per_build)
            .sum::<u32>();
        want_self.max(want_chain)
    }

    /// Consumers neither mastered nor held (built or pending), with the copies one build takes.
    fn open_consumers(&self, item: &str) -> impl Iterator<Item = &'a (String, u32)> {
        self.catalog
            .consumers_of(item)
            .iter()
            .filter(move |(consumer, _)| !self.closed(consumer) && self.have(consumer) == 0)
    }

    fn all_consumers_closed(&self, item: &str) -> bool {
        self.catalog
            .consumers_of(item)
            .iter()
            .all(|(consumer, _)| self.closed(consumer))
    }

    /// A blueprint counts its copies and pending builds; a part adds its blueprint sibling and
    /// that sibling's pending builds. An ingredient counts built plus pending whole items.
    fn held_of(&self, slot: &RecipeComponent) -> u32 {
        match slot.kind {
            ComponentKind::Blueprint => {
                self.holdings.quantity(&slot.path) + self.pending_path(&slot.path)
            }
            ComponentKind::Part => {
                let sibling = slot
                    .path
                    .strip_suffix("Component")
                    .map(|stem| format!("{stem}Blueprint"));
                self.holdings.quantity(&slot.path)
                    + sibling
                        .as_deref()
                        .map(|path| self.holdings.quantity(path))
                        .unwrap_or(0)
                    + sibling
                        .as_deref()
                        .map(|path| self.pending_path(path))
                        .unwrap_or(0)
            }
            ComponentKind::Ingredient => self.have(&slot.path),
        }
    }

    fn pending_path(&self, path: &str) -> u32 {
        self.facts.map_or(0, |facts| facts.pending(path))
    }
}
