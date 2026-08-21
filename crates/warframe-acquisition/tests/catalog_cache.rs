use std::{cell::Cell, fs};
use tempfile::tempdir;
use warframe_acquisition::{
    CatalogCache, CatalogCacheError, CatalogFetch, CatalogLoadSource, CatalogSource,
    RelicCatalogCache, RelicCatalogSource,
};

const VALID: &[u8] = br#"[{"uniqueName":"/Lotus/Powersuits/Test/Test","name":"Test Frame","type":"Warframe","category":"Warframes","masterable":true}]"#;
const VALID_RELICS: &[u8] = br#"[{"uniqueName":"/Lotus/Types/Game/Projections/TestABronze","rewards":[{"item":{"name":"Forma Blueprint"}}]}]"#;

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

impl RelicCatalogSource for Source {
    fn fetch(&self) -> Result<Vec<u8>, CatalogFetch> {
        CatalogSource::fetch(self)
    }
}

#[test]
fn relic_catalog_has_an_independent_atomic_cache_generation() {
    let dir = tempdir().unwrap();
    let cache = RelicCatalogCache::new(dir.path().join("catalog"));
    let loaded = cache
        .load(
            &Source {
                result: Ok(VALID_RELICS.to_vec()),
                calls: Cell::new(0),
            },
            100,
        )
        .unwrap();

    assert_eq!(loaded.source(), CatalogLoadSource::Network);
    assert_eq!(loaded.fetched_unix(), 100);
    assert!(dir.path().join("catalog/relic-generation.json").is_file());

    let stale = cache
        .load(
            &Source {
                result: Err(CatalogFetch::Unavailable),
                calls: Cell::new(0),
            },
            200,
        )
        .unwrap();
    assert_eq!(stale.source(), CatalogLoadSource::StaleCache);
    assert_eq!(stale.fetched_unix(), 100);
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
    assert!(dir.path().join("catalog/catalog-generation.json").is_file());
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
    assert!(
        loaded
            .index()
            .resolve("/Lotus/Powersuits/Test/Test")
            .is_some()
    );
}

#[test]
fn interrupted_temporary_generation_is_never_loaded() {
    let dir = tempdir().unwrap();
    let cache = CatalogCache::new(dir.path().join("catalog"));
    fs::create_dir_all(dir.path().join("catalog")).unwrap();
    fs::write(
        dir.path().join("catalog/catalog-generation.tmp-99"),
        b"partial",
    )
    .unwrap();
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

#[test]
fn mismatched_content_hash_rejects_the_entire_generation() {
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
    let path = dir.path().join("catalog/catalog-generation.json");
    let mut envelope: serde_json::Value =
        serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    envelope["fetched_unix"] = 999.into();
    fs::write(path, serde_json::to_vec(&envelope).unwrap()).unwrap();
    assert!(
        cache
            .load(
                &Source {
                    result: Err(CatalogFetch::Unavailable),
                    calls: Cell::new(0)
                },
                200
            )
            .is_err()
    );
}

#[test]
fn concurrent_writers_leave_one_whole_valid_generation() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("catalog");
    std::thread::scope(|scope| {
        for now in [100, 200] {
            let path = path.clone();
            scope.spawn(move || {
                let outcome = CatalogCache::new(path).load(
                    &Source {
                        result: Ok(VALID.to_vec()),
                        calls: Cell::new(0),
                    },
                    now,
                );
                // Either writer may lose the race to replace the generation file. On Windows
                // the replace of a destination being replaced at the same moment fails
                // transiently (MoveFileExW), and the loser reporting CacheWrite is a
                // legitimate outcome -- the invariant under test is that one whole valid
                // generation survives, asserted below. Any other failure is a bug.
                if let Err(error) = outcome
                    && !matches!(error, CatalogCacheError::CacheWrite)
                {
                    panic!("unexpected cache error under concurrent writers: {error}");
                }
            });
        }
    });
    let loaded = CatalogCache::new(path)
        .load(
            &Source {
                result: Err(CatalogFetch::Unavailable),
                calls: Cell::new(0),
            },
            300,
        )
        .unwrap();
    assert!(matches!(loaded.fetched_unix(), 100 | 200));
    assert!(
        loaded
            .index()
            .resolve("/Lotus/Powersuits/Test/Test")
            .is_some()
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
