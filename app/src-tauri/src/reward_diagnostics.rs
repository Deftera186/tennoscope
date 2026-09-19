//! Opt-in, local-only reward evidence. Only completed attempts are retained/exported; stopping or
//! starting a session invalidates an in-flight attempt without interrupting its OCR subprocess.

use std::{
    collections::VecDeque,
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
    process::Output,
    sync::{Mutex, MutexGuard, OnceLock},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use image::ImageEncoder;
use serde::Serialize;
use warframe_acquisition::RewardCatalogEntry;

use crate::reward_capture::CapturedFrame;

const MAX_SAMPLES: usize = 12;
const MAX_BYTES: usize = 64 * 1024 * 1024;
const MAX_TEXT: usize = 16 * 1024;
const MAX_METADATA: usize = 1024 * 1024;
const SAMPLE_INTERVAL: Duration = Duration::from_secs(3);
const RECORDING_DURATION: Duration = Duration::from_secs(600);
const BUILD_ID: &str = match option_env!("TENNOSCOPE_DIAGNOSTIC_BUILD") {
    Some(build) => build,
    None => "local diagnostic",
};

#[derive(Clone, Debug, Serialize)]
pub struct DiagnosticStatus {
    pub available: bool,
    pub recording: bool,
    pub samples: usize,
    pub build_id: String,
    pub message: String,
}

static RECORDER: OnceLock<Recorder> = OnceLock::new();

fn recorder() -> &'static Recorder {
    RECORDER.get_or_init(Recorder::default)
}

pub fn status() -> DiagnosticStatus {
    recorder().status(Instant::now())
}

pub fn start(app_data: &Path) -> Result<DiagnosticStatus, String> {
    recorder().start(app_data, Instant::now())
}

pub fn stop(reason: &str) -> DiagnosticStatus {
    let mut state = recorder().lock();
    state.freeze(reason);
    state.status()
}

/// Freeze recording and copy a snapshot of complete samples. Pending OCR is not awaited and its
/// incomplete evidence is discarded. No diagnostic images are added to report/clipboard text.
pub fn export_to(report_folder: &Path) -> Result<usize, String> {
    recorder().export_to(report_folder)
}

pub(crate) fn begin(
    frame: &CapturedFrame,
    candidates: &[RewardCatalogEntry],
) -> Option<Attempt<'static>> {
    recorder().begin(frame, candidates, Instant::now())
}

#[derive(Default)]
struct Recorder {
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    root: Option<PathBuf>,
    recording: bool,
    started: Option<Instant>,
    last_sample: Option<Instant>,
    next_id: u64,
    pending: Option<StoredSample>,
    completed: VecDeque<StoredSample>,
    bytes: usize,
    message: String,
}

struct StoredSample {
    id: u64,
    bytes: usize,
}

impl Recorder {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn status(&self, now: Instant) -> DiagnosticStatus {
        let mut state = self.lock();
        state.expire(now);
        state.status()
    }

    fn start(&self, app_data: &Path, now: Instant) -> Result<DiagnosticStatus, String> {
        if !cfg!(feature = "reward-diagnostics") {
            return Err("reward diagnostics are not available in this build".to_owned());
        }
        let mut state = self.lock();
        state.freeze("Starting a fresh diagnostic session");
        state.completed.clear();
        state.pending = None;
        state.bytes = 0;
        let root = app_data.join("reward-diagnostics-session");
        let result = (|| {
            if let Some(previous) = &state.root {
                remove_directory(previous)?;
            }
            if state.root.as_ref() != Some(&root) {
                remove_directory(&root)?;
            }
            fs::create_dir_all(&root).map_err(|error| error.to_string())
        })();
        if let Err(error) = result {
            state.fail(&format!("could not start reward diagnostics: {error}"));
            return Err(state.message.clone());
        }
        state.root = Some(root);
        state.started = Some(now);
        state.last_sample = None;
        state.recording = true;
        state.message =
            "Recording locally for up to 10 minutes; review images before sharing".to_owned();
        Ok(state.status())
    }

    fn begin(
        &self,
        frame: &CapturedFrame,
        candidates: &[RewardCatalogEntry],
        now: Instant,
    ) -> Option<Attempt<'_>> {
        if !cfg!(feature = "reward-diagnostics") {
            return None;
        }
        let mut state = self.lock();
        state.expire(now);
        if !state.recording
            || state.pending.is_some()
            || state
                .last_sample
                .is_some_and(|last| now.saturating_duration_since(last) < SAMPLE_INTERVAL)
        {
            return None;
        }
        state.next_id += 1;
        let id = state.next_id;
        state.last_sample = Some(now);
        state.pending = Some(StoredSample { id, bytes: 0 });
        let path = state.pending_path(id);
        let result = (|| {
            fs::create_dir(&path).map_err(|error| error.to_string())?;
            let image = &frame.image;
            let png = encode_png(
                image.as_bytes(),
                image.width(),
                image.height(),
                image.color().into(),
            )?;
            state.store_file(id, "frame.png", &png)
        })();
        if let Err(error) = result {
            state.fail(&error);
            return None;
        }
        Some(Attempt {
            recorder: self,
            id,
            active: true,
            sample: Sample {
                schema_version: 1,
                build_id: BUILD_ID,
                sample_id: id,
                started_unix_ms: unix_ms(),
                completed_unix_ms: 0,
                frame_backend: frame.frame_backend.label(),
                rect_origin: frame.rect_origin.label(),
                source_rect: [
                    i64::from(frame.rect.x),
                    i64::from(frame.rect.y),
                    i64::from(frame.rect.width),
                    i64::from(frame.rect.height),
                ],
                frame_size: [frame.image.width(), frame.image.height()],
                pool: candidates
                    .iter()
                    .take(512)
                    .map(|entry| bounded(&entry.name))
                    .collect(),
                pool_truncated: candidates.len() > 512,
                crops: Vec::new(),
                outcome: Outcome {
                    cards: None,
                    error: None,
                },
            },
        })
    }

    fn store_file(&self, id: u64, name: &str, bytes: &[u8]) -> bool {
        let mut state = self.lock();
        state.expire(Instant::now());
        if !state.current(id) {
            return false;
        }
        if let Err(error) = state.store_file(id, name, bytes) {
            state.fail(&error);
            return false;
        }
        true
    }

    fn export_to(&self, report_folder: &Path) -> Result<usize, String> {
        let mut state = self.lock();
        state.freeze("Recording stopped to save a completed diagnostic snapshot");
        if state.completed.is_empty() {
            return Ok(0);
        }
        // Holding this same lock throughout the copy prevents a concurrent start from deleting
        // the source, and prevents another export from publishing a partially copied directory.
        let destination = report_folder.join("reward-diagnostics");
        let staging = report_folder.join(".reward-diagnostics-export");
        let result = (|| {
            remove_directory(&staging)?;
            fs::create_dir(&staging).map_err(|error| error.to_string())?;
            for sample in &state.completed {
                let name = sample_name(sample.id);
                let source = state.root.as_ref().unwrap().join(&name);
                let target = staging.join(&name);
                fs::create_dir(&target).map_err(|error| error.to_string())?;
                for entry in fs::read_dir(source).map_err(|error| error.to_string())? {
                    let entry = entry.map_err(|error| error.to_string())?;
                    if !entry
                        .file_type()
                        .map_err(|error| error.to_string())?
                        .is_file()
                    {
                        return Err("unexpected non-file in a diagnostic sample".to_owned());
                    }
                    fs::copy(entry.path(), target.join(entry.file_name()))
                        .map_err(|error| error.to_string())?;
                }
            }
            remove_directory(&destination)?;
            fs::rename(&staging, &destination).map_err(|error| error.to_string())
        })();
        if let Err(error) = result {
            let cleanup = remove_directory(&staging);
            state.fail(&format!(
                "could not export reward diagnostics: {error}; cleanup: {cleanup:?}"
            ));
            return Err(state.message.clone());
        }
        Ok(state.completed.len())
    }
}

impl State {
    fn status(&self) -> DiagnosticStatus {
        let available = cfg!(feature = "reward-diagnostics");
        DiagnosticStatus {
            available,
            recording: self.recording,
            samples: self.completed.len(),
            build_id: BUILD_ID.to_owned(),
            message: if !available {
                "Reward diagnostics are not available in this build".to_owned()
            } else if self.message.is_empty() {
                "Not recording; diagnostic images are saved only after you start".to_owned()
            } else {
                self.message.clone()
            },
        }
    }

    fn current(&self, id: u64) -> bool {
        self.recording
            && self
                .pending
                .as_ref()
                .is_some_and(|pending| pending.id == id)
    }

    fn pending_path(&self, id: u64) -> PathBuf {
        self.root.as_ref().unwrap().join(format!(".pending-{id}"))
    }

    fn expire(&mut self, now: Instant) {
        if self.recording
            && self
                .started
                .is_some_and(|started| now.saturating_duration_since(started) >= RECORDING_DURATION)
        {
            self.freeze("The 10-minute diagnostic recording limit was reached");
        }
    }

    fn freeze(&mut self, reason: &str) {
        if self.recording || self.message.is_empty() {
            self.message = bounded(reason);
        }
        self.recording = false;
        if let Some(pending) = self.pending.take() {
            match remove_directory(&self.pending_path(pending.id)) {
                Ok(()) => self.bytes -= pending.bytes,
                Err(error) => {
                    self.message = bounded(&format!(
                        "Diagnostic recording stopped, but incomplete evidence could not be removed: {error}"
                    ));
                    log::warn!(
                        "Reward diagnostic recording stopped; incomplete evidence cleanup failed"
                    );
                }
            }
        }
    }

    fn fail(&mut self, reason: &str) {
        let message = bounded(&format!("Reward diagnostic recording failed: {reason}"));
        log::warn!("Reward diagnostic recording failed; see diagnostic status for details");
        self.message = message.clone();
        self.freeze(&message);
    }

    fn evict_oldest(&mut self) -> Result<(), String> {
        let oldest = self
            .completed
            .front()
            .ok_or("one diagnostic sample exceeds the 64 MiB limit")?;
        remove_directory(&self.root.as_ref().unwrap().join(sample_name(oldest.id)))?;
        self.bytes -= oldest.bytes;
        self.completed.pop_front();
        Ok(())
    }

    fn store_file(&mut self, id: u64, name: &str, bytes: &[u8]) -> Result<(), String> {
        if bytes.len() > MAX_BYTES {
            return Err("one diagnostic file exceeds the 64 MiB limit".to_owned());
        }
        while self.bytes > MAX_BYTES - bytes.len() {
            self.evict_oldest()?;
        }
        let path = self.pending_path(id).join(name);
        let mut file = fs::File::create_new(&path).map_err(|error| error.to_string())?;
        file.write_all(bytes).map_err(|error| error.to_string())?;
        self.pending.as_mut().unwrap().bytes += bytes.len();
        self.bytes += bytes.len();
        Ok(())
    }
}

fn remove_directory(path: &Path) -> Result<(), String> {
    match fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.to_string()),
    }
}

fn sample_name(id: u64) -> String {
    format!("sample-{id:06}")
}

fn unix_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

fn bounded(text: &str) -> String {
    let mut end = text.len().min(MAX_TEXT);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    let mut result = text[..end].to_owned();
    if end < text.len() {
        result.push_str("\n[truncated]");
    }
    result
}

struct BoundedBuffer {
    bytes: Vec<u8>,
    limit: usize,
}

impl Write for BoundedBuffer {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > self.limit.saturating_sub(self.bytes.len()) {
            return Err(io::Error::other("diagnostic data exceeds its size limit"));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn encode_png(
    pixels: &[u8],
    width: u32,
    height: u32,
    color: image::ExtendedColorType,
) -> Result<Vec<u8>, String> {
    let mut buffer = BoundedBuffer {
        bytes: Vec::new(),
        limit: MAX_BYTES,
    };
    image::codecs::png::PngEncoder::new(&mut buffer)
        .write_image(pixels, width, height, color)
        .map_err(|error| error.to_string())?;
    Ok(buffer.bytes)
}

#[derive(Serialize)]
struct Sample {
    schema_version: u8,
    build_id: &'static str,
    sample_id: u64,
    started_unix_ms: u128,
    completed_unix_ms: u128,
    frame_backend: &'static str,
    rect_origin: &'static str,
    source_rect: [i64; 4],
    frame_size: [u32; 2],
    pool: Vec<String>,
    pool_truncated: bool,
    crops: Vec<Crop>,
    outcome: Outcome,
}

#[derive(Serialize)]
struct Outcome {
    cards: Option<Vec<(String, f32)>>,
    error: Option<&'static str>,
}

#[derive(Serialize)]
struct Crop {
    layout: usize,
    slot: usize,
    rect: [u32; 4],
    png: String,
    started_unix_ms: u128,
    completed_unix_ms: u128,
    process: Option<OcrProcess>,
    matched: Option<(String, f32)>,
    error: Option<&'static str>,
}

#[derive(Serialize)]
struct OcrProcess {
    executable: String,
    tessdata_directory: Option<String>,
    page_segmentation_mode: String,
    success: bool,
    exit_code: Option<i32>,
    status: String,
    launch_error: Option<String>,
    stdout: String,
    stderr: String,
    stdout_truncated: bool,
    stderr_truncated: bool,
}

pub(crate) struct Attempt<'a> {
    recorder: &'a Recorder,
    id: u64,
    active: bool,
    sample: Sample,
}

impl Attempt<'_> {
    pub(crate) fn crop(
        &mut self,
        image: &image::GrayImage,
        layout: usize,
        slot: usize,
        rect: [u32; 4],
    ) -> Option<usize> {
        if !self.active {
            return None;
        }
        let index = self.sample.crops.len();
        let name = format!("crop-{index:02}-layout-{layout}-slot-{slot}.png");
        let png = match encode_png(
            image.as_raw(),
            image.width(),
            image.height(),
            image::ExtendedColorType::L8,
        ) {
            Ok(png) => png,
            Err(error) => {
                let mut state = self.recorder.lock();
                if state.current(self.id) {
                    state.fail(&error);
                }
                self.active = false;
                return None;
            }
        };
        self.active = self.recorder.store_file(self.id, &name, &png);
        if !self.active {
            return None;
        }
        self.sample.crops.push(Crop {
            layout,
            slot,
            rect,
            png: name,
            started_unix_ms: unix_ms(),
            completed_unix_ms: 0,
            process: None,
            matched: None,
            error: None,
        });
        Some(index)
    }

    pub(crate) fn process(
        &mut self,
        index: usize,
        program: &Path,
        data: Option<&Path>,
        psm: &str,
        result: &io::Result<Output>,
    ) {
        let (success, exit_code, status, launch_error, stdout, stderr) = match result {
            Ok(output) => (
                output.status.success(),
                output.status.code(),
                output.status.to_string(),
                None,
                bounded(&String::from_utf8_lossy(
                    &output.stdout[..output.stdout.len().min(MAX_TEXT)],
                )),
                bounded(&String::from_utf8_lossy(
                    &output.stderr[..output.stderr.len().min(MAX_TEXT)],
                )),
            ),
            Err(error) => (
                false,
                None,
                "launch failed".to_owned(),
                Some(bounded(&error.to_string())),
                String::new(),
                String::new(),
            ),
        };
        self.sample.crops[index].process = Some(OcrProcess {
            executable: bounded(&program.to_string_lossy()),
            tessdata_directory: data.map(|path| bounded(&path.to_string_lossy())),
            page_segmentation_mode: bounded(psm),
            success,
            exit_code,
            status,
            launch_error,
            stdout,
            stderr,
            stdout_truncated: result
                .as_ref()
                .is_ok_and(|output| output.stdout.len() > MAX_TEXT),
            stderr_truncated: result
                .as_ref()
                .is_ok_and(|output| output.stderr.len() > MAX_TEXT),
        });
    }

    pub(crate) fn crop_outcome(
        &mut self,
        index: usize,
        matched: Option<&(String, f32)>,
        error: Option<&'static str>,
    ) {
        let crop = &mut self.sample.crops[index];
        crop.completed_unix_ms = unix_ms();
        crop.matched = matched.map(|(name, score)| (bounded(name), *score));
        crop.error = error;
    }

    pub(crate) fn finish(mut self, result: &Result<Vec<(String, f32)>, &'static str>) {
        let mut state = self.recorder.lock();
        state.expire(Instant::now());
        if !state.current(self.id) {
            return;
        }
        self.sample.completed_unix_ms = unix_ms();
        self.sample.outcome = match result {
            Ok(cards) => Outcome {
                cards: Some(
                    cards
                        .iter()
                        .map(|(name, score)| (bounded(name), *score))
                        .collect(),
                ),
                error: None,
            },
            Err(reason) => Outcome {
                cards: None,
                error: Some(reason),
            },
        };
        let result = (|| {
            let mut buffer = BoundedBuffer {
                bytes: Vec::new(),
                limit: MAX_METADATA,
            };
            serde_json::to_writer_pretty(&mut buffer, &self.sample)
                .map_err(|error| error.to_string())?;
            state.store_file(self.id, "sample.json", &buffer.bytes)?;
            while state.completed.len() >= MAX_SAMPLES {
                state.evict_oldest()?;
            }
            fs::rename(
                state.pending_path(self.id),
                state.root.as_ref().unwrap().join(sample_name(self.id)),
            )
            .map_err(|error| error.to_string())?;
            let completed = state.pending.take().unwrap();
            state.completed.push_back(completed);
            Ok::<(), String>(())
        })();
        if let Err(error) = result {
            state.fail(&error);
        }
    }
}

impl Drop for Attempt<'_> {
    fn drop(&mut self) {
        let mut state = self.recorder.lock();
        if state.current(self.id) {
            state.fail("an OCR attempt ended before its result was recorded");
        }
    }
}

#[cfg(all(test, feature = "reward-diagnostics"))]
mod tests {
    use super::*;
    use crate::reward_capture::{CapturedFrame, FrameBackend, RectOrigin};

    fn frame(tag: u8) -> CapturedFrame {
        CapturedFrame {
            rect: crate::overlay_window::WindowRect {
                x: -1920,
                y: 0,
                width: 1920,
                height: 1080,
            },
            image: image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
                8,
                8,
                image::Rgba([tag, 0, 0, 255]),
            )),
            rect_origin: RectOrigin::X11,
            frame_backend: FrameBackend::X11,
        }
    }

    fn pool() -> Vec<RewardCatalogEntry> {
        vec![RewardCatalogEntry {
            name: "Forma Blueprint".to_owned(),
            ducats: 0,
        }]
    }

    #[test]
    fn consent_and_rate_limit_prevent_capture_files() {
        let storage = tempfile::tempdir().unwrap();
        let recorder = Recorder::default();
        let now = Instant::now();
        assert!(recorder.begin(&frame(1), &pool(), now).is_none());
        assert_eq!(std::fs::read_dir(storage.path()).unwrap().count(), 0);
        recorder.start(storage.path(), now).unwrap();
        let first = recorder.begin(&frame(1), &pool(), now).unwrap();
        assert!(
            recorder
                .begin(&frame(2), &pool(), now + Duration::from_secs(3))
                .is_none()
        );
        first.finish(&Err("blank"));
        assert!(
            recorder
                .begin(&frame(2), &pool(), now + Duration::from_secs(2))
                .is_none()
        );
        assert!(
            recorder
                .begin(&frame(2), &pool(), now + Duration::from_secs(3))
                .is_some()
        );
    }

    #[test]
    fn export_freezes_completed_samples_and_old_attempt_cannot_join_new_session() {
        let storage = tempfile::tempdir().unwrap();
        let report = tempfile::tempdir().unwrap();
        let recorder = Recorder::default();
        let now = Instant::now();
        recorder.start(storage.path(), now).unwrap();
        recorder
            .begin(&frame(11), &pool(), now)
            .unwrap()
            .finish(&Err("blank"));
        let old = recorder
            .begin(&frame(22), &pool(), now + Duration::from_secs(3))
            .unwrap();
        std::thread::scope(|threads| {
            let (release, wait) = std::sync::mpsc::channel();
            let old_ocr = threads.spawn(move || {
                wait.recv().unwrap();
                old.finish(&Ok(vec![("Old session".to_owned(), 1.0)]));
            });
            assert_eq!(recorder.export_to(report.path()).unwrap(), 1);
            assert!(!recorder.status(Instant::now()).recording);
            recorder.start(storage.path(), now).unwrap();
            let new = recorder.begin(&frame(33), &pool(), now).unwrap();
            release.send(()).unwrap();
            new.finish(&Ok(vec![("Forma Blueprint".to_owned(), 1.0)]));
            old_ocr.join().unwrap();
        });
        assert_eq!(recorder.export_to(report.path()).unwrap(), 1);
        let samples: Vec<_> = std::fs::read_dir(report.path().join("reward-diagnostics"))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        assert_eq!(samples.len(), 1);
        let pixels = image::open(samples[0].path().join("frame.png"))
            .unwrap()
            .to_rgba8();
        assert_eq!(pixels.get_pixel(0, 0)[0], 33);
        let metadata: serde_json::Value =
            serde_json::from_slice(&std::fs::read(samples[0].path().join("sample.json")).unwrap())
                .unwrap();
        assert_eq!(metadata["outcome"]["cards"][0][0], "Forma Blueprint");
    }

    #[test]
    fn retention_keeps_newest_twelve_complete_attempts_and_expires() {
        let storage = tempfile::tempdir().unwrap();
        let report = tempfile::tempdir().unwrap();
        let recorder = Recorder::default();
        let now = Instant::now();
        recorder.start(storage.path(), now).unwrap();
        for index in 0..14 {
            recorder
                .begin(
                    &frame(index),
                    &pool(),
                    now + Duration::from_secs(u64::from(index) * 3),
                )
                .unwrap()
                .finish(&Err("blank"));
        }
        assert!(
            recorder
                .begin(&frame(99), &pool(), now + Duration::from_secs(600))
                .is_none()
        );
        assert!(!recorder.status(now + Duration::from_secs(600)).recording);
        assert_eq!(recorder.export_to(report.path()).unwrap(), 12);
        let mut retained: Vec<_> = std::fs::read_dir(report.path().join("reward-diagnostics"))
            .unwrap()
            .map(|entry| {
                image::open(entry.unwrap().path().join("frame.png"))
                    .unwrap()
                    .to_rgba8()
                    .get_pixel(0, 0)[0]
            })
            .collect();
        retained.sort_unstable();
        assert_eq!(retained, (2..14).collect::<Vec<u8>>());
    }

    #[test]
    fn completed_evidence_preserves_crop_pixels_and_bounded_engine_failure_details() {
        let storage = tempfile::tempdir().unwrap();
        let report = tempfile::tempdir().unwrap();
        let recorder = Recorder::default();
        let now = Instant::now();
        recorder.start(storage.path(), now).unwrap();
        let mut attempt = recorder.begin(&frame(7), &pool(), now).unwrap();
        let crop = image::GrayImage::from_pixel(3, 2, image::Luma([123]));
        let index = attempt.crop(&crop, 4, 0, [478, 408, 240, 58]).unwrap();
        let failure = Err(io::Error::other("x".repeat(MAX_TEXT * 2)));
        attempt.process(
            index,
            Path::new("engine/tesseract"),
            Some(Path::new("engine")),
            "11",
            &failure,
        );
        attempt.crop_outcome(index, None, Some("tesseract is not available"));
        attempt.finish(&Err("tesseract is not available"));
        assert_eq!(recorder.export_to(report.path()).unwrap(), 1);
        let sample = std::fs::read_dir(report.path().join("reward-diagnostics"))
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        let metadata: serde_json::Value =
            serde_json::from_slice(&std::fs::read(sample.join("sample.json")).unwrap()).unwrap();
        let recorded_crop = &metadata["crops"][0];
        assert_eq!(
            image::open(sample.join(recorded_crop["png"].as_str().unwrap()))
                .unwrap()
                .to_luma8(),
            crop
        );
        assert_eq!(
            metadata["source_rect"],
            serde_json::json!([-1920, 0, 1920, 1080])
        );
        assert_eq!(
            recorded_crop["rect"],
            serde_json::json!([478, 408, 240, 58])
        );
        assert_eq!(recorded_crop["process"]["success"], false);
        assert_eq!(
            recorded_crop["process"]["exit_code"],
            serde_json::Value::Null
        );
        assert!(
            recorded_crop["process"]["launch_error"]
                .as_str()
                .unwrap()
                .len()
                < MAX_TEXT + 32
        );
        assert_eq!(metadata["outcome"]["error"], "tesseract is not available");
    }

    #[test]
    fn byte_budget_evicts_whole_samples_and_oversize_capture_stops_recording() {
        let storage = tempfile::tempdir().unwrap();
        let report = tempfile::tempdir().unwrap();
        let recorder = Recorder::default();
        let now = Instant::now();
        let payload = vec![0; 33 * 1024 * 1024];
        recorder.start(storage.path(), now).unwrap();
        for tag in 1..=2 {
            let attempt = recorder
                .begin(
                    &frame(tag),
                    &pool(),
                    now + Duration::from_secs(u64::from(tag) * 3),
                )
                .unwrap();
            assert!(recorder.store_file(attempt.id, "large-crop.png", &payload));
            attempt.finish(&Err("blank"));
        }
        assert_eq!(recorder.export_to(report.path()).unwrap(), 1);
        let retained = std::fs::read_dir(report.path().join("reward-diagnostics"))
            .unwrap()
            .next()
            .unwrap()
            .unwrap();
        let pixels = image::open(retained.path().join("frame.png"))
            .unwrap()
            .to_rgba8();
        assert_eq!(pixels.get_pixel(0, 0)[0], 2);
        recorder.start(storage.path(), now).unwrap();
        let oversize = recorder.begin(&frame(3), &pool(), now).unwrap();
        assert!(recorder.store_file(oversize.id, "first-crop.png", &payload));
        assert!(!recorder.store_file(oversize.id, "second-crop.png", &payload));
        assert!(!recorder.status(now).recording);
        assert_eq!(
            std::fs::read_dir(storage.path().join("reward-diagnostics-session"))
                .unwrap()
                .count(),
            0
        );
        let failure = recorder.status(now).message;
        assert_eq!(recorder.export_to(report.path()).unwrap(), 0);
        assert_eq!(recorder.status(now).message, failure);
        recorder.lock().freeze("Warframe exited");
        assert_eq!(recorder.status(now).message, failure);
    }
}

#[cfg(all(test, not(feature = "reward-diagnostics")))]
mod unavailable_tests {
    #[test]
    fn unavailable_recording_cannot_write_or_export() {
        let storage = tempfile::tempdir().unwrap();
        assert!(super::start(storage.path()).is_err());
        assert_eq!(super::export_to(storage.path()).unwrap(), 0);
        assert_eq!(std::fs::read_dir(storage.path()).unwrap().count(), 0);
    }
}
