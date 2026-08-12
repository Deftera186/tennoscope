//! The decode path must log the *real* error before swallowing it into the
//! generic SnapshotInvalid, and must never log payload contents.

use std::sync::{Arc, Mutex};

use warframe_acquisition::{InventoryJsonDecoder, SnapshotDecoder};

struct CaptureLog {
    lines: Arc<Mutex<Vec<String>>>,
}

impl log::Log for CaptureLog {
    fn enabled(&self, _metadata: &log::Metadata<'_>) -> bool {
        true
    }
    fn log(&self, record: &log::Record<'_>) {
        self.lines
            .lock()
            .expect("lock")
            .push(format!("{} {}", record.level(), record.args()));
    }
    fn flush(&self) {}
}

fn install_capture_logger() -> Arc<Mutex<Vec<String>>> {
    static LINES: std::sync::OnceLock<Arc<Mutex<Vec<String>>>> = std::sync::OnceLock::new();
    LINES
        .get_or_init(|| {
            let lines = Arc::new(Mutex::new(Vec::<String>::new()));
            log::set_boxed_logger(Box::new(CaptureLog {
                lines: Arc::clone(&lines),
            }))
            .expect("logger installs once");
            log::set_max_level(log::LevelFilter::Debug);
            lines
        })
        .clone()
}

#[test]
fn decode_failure_logs_the_real_serde_error_and_length_but_not_contents() {
    let lines = install_capture_logger();
    lines.lock().expect("lock").clear();
    let payload = br#"{"LastInventorySync":null,"Suits":,"broken"}"#;
    let decoder = InventoryJsonDecoder::default();

    let result = decoder.decode(payload);

    assert!(matches!(
        result,
        Err(warframe_acquisition::AcquisitionError::SnapshotInvalid)
    ));
    let captured = lines.lock().expect("lock").clone();
    assert!(
        captured
            .iter()
            .any(|line| line.contains("inventory decode failed")),
        "a decode failure line must be logged, got: {captured:?}"
    );
    let line = captured
        .iter()
        .find(|line| line.contains("inventory decode failed"))
        .expect("line exists");
    assert!(
        line.contains("payload_bytes=44"),
        "payload length must be logged: {line}"
    );
    assert!(
        !line.contains(r#""Suits""#),
        "payload contents must never be logged: {line}"
    );
}

#[test]
fn missing_sync_timestamp_is_logged_without_contents() {
    let lines = install_capture_logger();
    lines.lock().expect("lock").clear();
    let payload = br#"{"LastInventorySync":null,"Suits":[],"LongGuns":[],"Pistols":[],"Melee":[],"Sentinels":[],"MiscItems":[],"Recipes":[],"PendingRecipes":[],"XPInfo":[],"SpaceSuits":[],"SpaceMelee":[],"SpaceGuns":[],"SentinelWeapons":[],"KubrowPets":[],"OperatorAmps":[],"MechSuits":[]}"#;
    let decoder = InventoryJsonDecoder::default();

    let result = decoder.decode(payload);

    assert!(result.is_err());
    let captured = lines.lock().expect("lock").clone();
    let line = captured
        .iter()
        .find(|line| line.contains("inventory sync timestamp missing"))
        .expect("a missing-timestamp line must be logged");
    assert!(line.contains("payload_bytes=267"));
    assert!(!line.contains("Suits"));
}
