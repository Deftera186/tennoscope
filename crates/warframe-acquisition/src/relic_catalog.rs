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
                RewardNeedle::from_paths(name.clone(), catalog.paths_for_name(name)).ok()
            })
            .collect()
    }
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
