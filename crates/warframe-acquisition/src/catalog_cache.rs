use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    time::Duration,
};

use reqwest::blocking::Client;
use thiserror::Error;

use crate::CatalogIndex;

pub const WFCD_ALL_JSON_URL: &str = "https://raw.githubusercontent.com/WFCD/warframe-items/81c893536dee6de23fbf114cf52d1b01d23bd65d/data/json/All.json";
const MAX_CATALOG_BYTES: usize = 64 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CatalogFetch {
    Unavailable,
    TooLarge,
}

pub trait CatalogSource {
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
            if let Ok(index) = CatalogIndex::from_wfcd_json(&bytes) {
                self.store(&bytes, now_unix)?;
                return Ok(CatalogLoad {
                    index,
                    source: CatalogLoadSource::Network,
                    fetched_unix: now_unix,
                });
            }
        }
        self.load_cached()
    }

    fn store(&self, bytes: &[u8], fetched_unix: u64) -> Result<(), CatalogCacheError> {
        fs::create_dir_all(&self.directory).map_err(|_| CatalogCacheError::CacheWrite)?;
        atomic_write(&self.directory.join("All.json"), bytes)?;
        atomic_write(
            &self.directory.join("metadata"),
            fetched_unix.to_string().as_bytes(),
        )?;
        Ok(())
    }

    fn load_cached(&self) -> Result<CatalogLoad, CatalogCacheError> {
        let bytes = fs::read(self.directory.join("All.json"))
            .map_err(|_| CatalogCacheError::Unavailable)?;
        let fetched_unix = fs::read_to_string(self.directory.join("metadata"))
            .ok()
            .and_then(|value| value.parse().ok())
            .ok_or(CatalogCacheError::Unavailable)?;
        let index =
            CatalogIndex::from_wfcd_json(&bytes).map_err(|_| CatalogCacheError::Unavailable)?;
        Ok(CatalogLoad {
            index,
            source: CatalogLoadSource::StaleCache,
            fetched_unix,
        })
    }
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), CatalogCacheError> {
    let temporary = path.with_extension("tmp");
    fs::write(&temporary, bytes).map_err(|_| CatalogCacheError::CacheWrite)?;
    fs::rename(temporary, path).map_err(|_| CatalogCacheError::CacheWrite)
}
