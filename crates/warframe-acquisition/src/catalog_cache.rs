use std::{
    fs,
    io::{Read, Write},
    path::PathBuf,
    time::Duration,
};

use atomicwrites::{AtomicFile, OverwriteBehavior};
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::{CatalogIndex, RelicRewardIndex};

pub const WFCD_ALL_JSON_URL: &str = "https://raw.githubusercontent.com/WFCD/warframe-items/81c893536dee6de23fbf114cf52d1b01d23bd65d/data/json/All.json";
pub const WFCD_RELICS_JSON_URL: &str = "https://raw.githubusercontent.com/WFCD/warframe-items/81c893536dee6de23fbf114cf52d1b01d23bd65d/data/json/Relics.json";
const MAX_CATALOG_BYTES: usize = 64 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CatalogFetch {
    Unavailable,
    TooLarge,
}
pub trait CatalogSource {
    fn fetch(&self) -> Result<Vec<u8>, CatalogFetch>;
}

pub trait RelicCatalogSource {
    fn fetch(&self) -> Result<Vec<u8>, CatalogFetch>;
}

pub struct WfcdCatalogHttp {
    client: Client,
}
impl WfcdCatalogHttp {
    pub fn new() -> Result<Self, CatalogFetch> {
        Ok(Self {
            client: Client::builder()
                .connect_timeout(Duration::from_secs(5))
                .timeout(Duration::from_secs(30))
                .build()
                .map_err(|_| CatalogFetch::Unavailable)?,
        })
    }
}
impl CatalogSource for WfcdCatalogHttp {
    fn fetch(&self) -> Result<Vec<u8>, CatalogFetch> {
        let response = self
            .client
            .get(WFCD_ALL_JSON_URL)
            .send()
            .map_err(|_| CatalogFetch::Unavailable)?;
        if !response.status().is_success() {
            return Err(CatalogFetch::Unavailable);
        }
        let mut bytes = Vec::new();
        response
            .take((MAX_CATALOG_BYTES + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(|_| CatalogFetch::Unavailable)?;
        if bytes.len() > MAX_CATALOG_BYTES {
            return Err(CatalogFetch::TooLarge);
        }
        Ok(bytes)
    }
}

pub struct WfcdRelicCatalogHttp {
    client: Client,
}

impl WfcdRelicCatalogHttp {
    pub fn new() -> Result<Self, CatalogFetch> {
        Ok(Self {
            client: Client::builder()
                .connect_timeout(Duration::from_secs(5))
                .timeout(Duration::from_secs(30))
                .build()
                .map_err(|_| CatalogFetch::Unavailable)?,
        })
    }
}

impl RelicCatalogSource for WfcdRelicCatalogHttp {
    fn fetch(&self) -> Result<Vec<u8>, CatalogFetch> {
        fetch_bounded(&self.client, WFCD_RELICS_JSON_URL)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CatalogLoadSource {
    Network,
    StaleCache,
}
pub struct CatalogLoad {
    index: CatalogIndex,
    source: CatalogLoadSource,
    fetched_unix: u64,
}
impl CatalogLoad {
    pub const fn index(&self) -> &CatalogIndex {
        &self.index
    }
    pub const fn source(&self) -> CatalogLoadSource {
        self.source
    }
    pub const fn fetched_unix(&self) -> u64 {
        self.fetched_unix
    }
}

#[derive(Debug, Error)]
pub enum CatalogCacheError {
    #[error("no valid Warframe catalog is available")]
    Unavailable,
    #[error("catalog cache could not be updated")]
    CacheWrite,
}

#[derive(Serialize, Deserialize)]
struct Generation {
    fetched_unix: u64,
    catalog: serde_json::Value,
    content_hash: String,
}

pub struct RelicCatalogLoad {
    index: RelicRewardIndex,
    source: CatalogLoadSource,
    fetched_unix: u64,
}

impl RelicCatalogLoad {
    pub const fn index(&self) -> &RelicRewardIndex {
        &self.index
    }

    pub const fn source(&self) -> CatalogLoadSource {
        self.source
    }

    pub const fn fetched_unix(&self) -> u64 {
        self.fetched_unix
    }
}

pub struct CatalogCache {
    directory: PathBuf,
}
impl CatalogCache {
    pub fn new(directory: impl Into<PathBuf>) -> Self {
        Self {
            directory: directory.into(),
        }
    }
    pub fn load(
        &self,
        source: &dyn CatalogSource,
        now_unix: u64,
    ) -> Result<CatalogLoad, CatalogCacheError> {
        if let Ok(bytes) = source.fetch() {
            if let (Ok(index), Ok(catalog)) = (
                CatalogIndex::from_wfcd_json(&bytes),
                serde_json::from_slice(&bytes),
            ) {
                self.store(catalog, now_unix)?;
                return Ok(CatalogLoad {
                    index,
                    source: CatalogLoadSource::Network,
                    fetched_unix: now_unix,
                });
            }
        }
        self.load_cached()
    }

    fn store(
        &self,
        catalog: serde_json::Value,
        fetched_unix: u64,
    ) -> Result<(), CatalogCacheError> {
        fs::create_dir_all(&self.directory).map_err(|_| CatalogCacheError::CacheWrite)?;
        let content_hash = generation_hash(fetched_unix, &catalog)?;
        let bytes = serde_json::to_vec(&Generation {
            fetched_unix,
            catalog,
            content_hash,
        })
        .map_err(|_| CatalogCacheError::CacheWrite)?;
        let final_path = self.directory.join("catalog-generation.json");
        AtomicFile::new(final_path, OverwriteBehavior::AllowOverwrite)
            .write(|file| file.write_all(&bytes).and_then(|_| file.sync_all()))
            .map_err(|_| CatalogCacheError::CacheWrite)
    }

    pub fn load_cached(&self) -> Result<CatalogLoad, CatalogCacheError> {
        let bytes = fs::read(self.directory.join("catalog-generation.json"))
            .map_err(|_| CatalogCacheError::Unavailable)?;
        let generation: Generation =
            serde_json::from_slice(&bytes).map_err(|_| CatalogCacheError::Unavailable)?;
        if generation.content_hash
            != generation_hash(generation.fetched_unix, &generation.catalog)
                .map_err(|_| CatalogCacheError::Unavailable)?
        {
            return Err(CatalogCacheError::Unavailable);
        }
        let catalog_bytes =
            serde_json::to_vec(&generation.catalog).map_err(|_| CatalogCacheError::Unavailable)?;
        let index = CatalogIndex::from_wfcd_json(&catalog_bytes)
            .map_err(|_| CatalogCacheError::Unavailable)?;
        Ok(CatalogLoad {
            index,
            source: CatalogLoadSource::StaleCache,
            fetched_unix: generation.fetched_unix,
        })
    }
}

pub struct RelicCatalogCache {
    directory: PathBuf,
}

impl RelicCatalogCache {
    pub fn new(directory: impl Into<PathBuf>) -> Self {
        Self {
            directory: directory.into(),
        }
    }

    pub fn load(
        &self,
        source: &dyn RelicCatalogSource,
        now_unix: u64,
    ) -> Result<RelicCatalogLoad, CatalogCacheError> {
        if let Ok(bytes) = source.fetch() {
            if let (Ok(index), Ok(catalog)) = (
                RelicRewardIndex::from_wfcd_json(&bytes),
                serde_json::from_slice(&bytes),
            ) {
                self.store(catalog, now_unix)?;
                return Ok(RelicCatalogLoad {
                    index,
                    source: CatalogLoadSource::Network,
                    fetched_unix: now_unix,
                });
            }
        }
        self.load_cached()
    }

    fn store(
        &self,
        catalog: serde_json::Value,
        fetched_unix: u64,
    ) -> Result<(), CatalogCacheError> {
        fs::create_dir_all(&self.directory).map_err(|_| CatalogCacheError::CacheWrite)?;
        let content_hash = generation_hash(fetched_unix, &catalog)?;
        let bytes = serde_json::to_vec(&Generation {
            fetched_unix,
            catalog,
            content_hash,
        })
        .map_err(|_| CatalogCacheError::CacheWrite)?;
        AtomicFile::new(
            self.directory.join("relic-generation.json"),
            OverwriteBehavior::AllowOverwrite,
        )
        .write(|file| file.write_all(&bytes).and_then(|_| file.sync_all()))
        .map_err(|_| CatalogCacheError::CacheWrite)
    }

    pub fn load_cached(&self) -> Result<RelicCatalogLoad, CatalogCacheError> {
        let bytes = fs::read(self.directory.join("relic-generation.json"))
            .map_err(|_| CatalogCacheError::Unavailable)?;
        let generation: Generation =
            serde_json::from_slice(&bytes).map_err(|_| CatalogCacheError::Unavailable)?;
        if generation.content_hash
            != generation_hash(generation.fetched_unix, &generation.catalog)
                .map_err(|_| CatalogCacheError::Unavailable)?
        {
            return Err(CatalogCacheError::Unavailable);
        }
        let catalog_bytes =
            serde_json::to_vec(&generation.catalog).map_err(|_| CatalogCacheError::Unavailable)?;
        let index = RelicRewardIndex::from_wfcd_json(&catalog_bytes)
            .map_err(|_| CatalogCacheError::Unavailable)?;
        Ok(RelicCatalogLoad {
            index,
            source: CatalogLoadSource::StaleCache,
            fetched_unix: generation.fetched_unix,
        })
    }
}

fn fetch_bounded(client: &Client, url: &str) -> Result<Vec<u8>, CatalogFetch> {
    let response = client
        .get(url)
        .send()
        .map_err(|_| CatalogFetch::Unavailable)?;
    if !response.status().is_success() {
        return Err(CatalogFetch::Unavailable);
    }
    let mut bytes = Vec::new();
    response
        .take((MAX_CATALOG_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| CatalogFetch::Unavailable)?;
    if bytes.len() > MAX_CATALOG_BYTES {
        return Err(CatalogFetch::TooLarge);
    }
    Ok(bytes)
}

fn generation_hash(
    fetched_unix: u64,
    catalog: &serde_json::Value,
) -> Result<String, CatalogCacheError> {
    let mut digest = Sha256::new();
    digest.update(fetched_unix.to_le_bytes());
    digest.update(serde_json::to_vec(catalog).map_err(|_| CatalogCacheError::CacheWrite)?);
    Ok(digest
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}
