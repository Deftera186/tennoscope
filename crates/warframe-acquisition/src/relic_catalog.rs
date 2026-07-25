use std::collections::{BTreeMap, BTreeSet};

use serde::Deserialize;

use crate::{CatalogError, CatalogIndex, RewardNeedle};

#[derive(Clone, Debug, Default)]
pub struct RelicRewardIndex {
    rewards: BTreeMap<String, BTreeSet<String>>,
}

impl RelicRewardIndex {
    pub fn from_wfcd_json(bytes: &[u8]) -> Result<Self, CatalogError> {
        let records: Vec<RelicRecord> =
            serde_json::from_slice(bytes).map_err(|_| CatalogError::InvalidJson)?;
        let mut rewards = BTreeMap::new();
        for record in records {
            if !record
                .unique_name
                .starts_with("/Lotus/Types/Game/Projections/")
            {
                continue;
            }
            let names = record
                .rewards
                .into_iter()
                .map(|reward| reward.item.name.trim().to_owned())
                .filter(|name| !name.is_empty())
                .collect::<BTreeSet<_>>();
            if !names.is_empty() {
                rewards.insert(record.unique_name, names);
            }
        }
        Ok(Self { rewards })
    }

    pub fn candidates_for_projection_paths(
        &self,
        projection_paths: &[String],
        catalog: &CatalogIndex,
    ) -> Vec<RewardNeedle> {
        projection_paths
            .iter()
            .filter_map(|path| self.rewards.get(path))
            .flatten()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .filter_map(|name| {
                RewardNeedle::from_paths(name.clone(), reward_catalog_paths(name, catalog)).ok()
            })
            .collect()
    }
}

fn reward_catalog_paths(name: &str, catalog: &CatalogIndex) -> Vec<String> {
    let exact = catalog.paths_for_name(name);
    if !exact.is_empty() {
        return exact;
    }

    let without_quantity = name.split_once(' ').and_then(|(quantity, item)| {
        quantity
            .strip_suffix('X')
            .filter(|count| !count.is_empty() && count.bytes().all(|byte| byte.is_ascii_digit()))
            .map(|_| item)
    });
    for alias in [without_quantity, name.strip_suffix(" Blueprint")]
        .into_iter()
        .flatten()
    {
        let paths = catalog.paths_for_name(alias);
        if !paths.is_empty() {
            return paths;
        }
        if let Some(component_name) = alias.strip_suffix(" Blueprint") {
            let paths = catalog.paths_for_name(component_name);
            if !paths.is_empty() {
                return paths;
            }
        }
    }
    Vec::new()
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RelicRecord {
    unique_name: String,
    #[serde(default)]
    rewards: Vec<RelicReward>,
}

#[derive(Deserialize)]
struct RelicReward {
    item: RelicRewardItem,
}

#[derive(Deserialize)]
struct RelicRewardItem {
    name: String,
}
