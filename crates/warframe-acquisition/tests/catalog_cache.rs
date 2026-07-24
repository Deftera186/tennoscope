use std::{cell::Cell, fs};
use tempfile::tempdir;
use warframe_acquisition::{CatalogCache, CatalogFetch, CatalogLoadSource, CatalogSource};

const VALID: &[u8] = br#"[{"uniqueName":"/Lotus/Powersuits/Test/Test","name":"Test Frame","type":"Warframe","category":"Warframes","masterable":true}]"#;

struct Source {
    result: Result<Vec<u8>, CatalogFetch>,
    calls: Cell<usize>,
}
impl CatalogSource for Source {
    fn fetch(&self) -> Result<Vec<u8>, CatalogFetch> {
        self.calls.set(self.calls.get() + 1);
        self.result.clone()
    }
}

#[test]
fn valid_download_is_atomically_cached_with_freshness_metadata() {
    let dir = tempdir().unwrap();
    let cache = CatalogCache::new(dir.path().join("catalog"));
    let loaded = cache
        .load(
            &Source {
                result: Ok(VALID.to_vec()),
                calls: Cell::new(0),
            },
            100,
        )
        .unwrap();
    assert_eq!(loaded.source(), CatalogLoadSource::Network);
    assert_eq!(loaded.fetched_unix(), 100);
    assert!(
        loaded
            .index()
            .resolve("/Lotus/Powersuits/Test/Test")
            .is_some()
    );
    assert_eq!(
        fs::read(dir.path().join("catalog/All.json")).unwrap(),
        VALID
    );
}

#[test]
fn stale_complete_cache_is_used_when_network_fails() {
    let dir = tempdir().unwrap();
    let cache = CatalogCache::new(dir.path().join("catalog"));
    cache
        .load(
            &Source {
                result: Ok(VALID.to_vec()),
                calls: Cell::new(0),
            },
            100,
        )
        .unwrap();
    let loaded = cache
        .load(
            &Source {
                result: Err(CatalogFetch::Unavailable),
                calls: Cell::new(0),
            },
            200,
        )
        .unwrap();
    assert_eq!(loaded.source(), CatalogLoadSource::StaleCache);
    assert_eq!(loaded.fetched_unix(), 100);
}

#[test]
fn invalid_download_never_replaces_the_last_complete_catalog() {
    let dir = tempdir().unwrap();
    let cache = CatalogCache::new(dir.path().join("catalog"));
    cache
        .load(
            &Source {
                result: Ok(VALID.to_vec()),
                calls: Cell::new(0),
            },
            100,
        )
        .unwrap();
    let loaded = cache
        .load(
            &Source {
                result: Ok(b"not json".to_vec()),
                calls: Cell::new(0),
            },
            200,
        )
        .unwrap();
    assert_eq!(loaded.source(), CatalogLoadSource::StaleCache);
    assert_eq!(
        fs::read(dir.path().join("catalog/All.json")).unwrap(),
        VALID
    );
}

#[test]
fn fails_honestly_without_network_or_a_valid_cache() {
    let dir = tempdir().unwrap();
    let cache = CatalogCache::new(dir.path().join("catalog"));
    assert!(
        cache
            .load(
                &Source {
                    result: Err(CatalogFetch::Unavailable),
                    calls: Cell::new(0)
                },
                100
            )
            .is_err()
    );
}
