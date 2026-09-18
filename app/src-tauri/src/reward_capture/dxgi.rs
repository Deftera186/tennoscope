//! Serialized Windows desktop capture, leased only by readers that have requested pixels.

use std::sync::Mutex;
use std::time::Duration;

use super::availability::RetryCooldown;
use super::{GameFrameSource, MonitorFrame};
use crate::overlay_window::WindowRect;

const CAPTURE_RETRY_INTERVAL: Duration = Duration::from_secs(5);
const ERR_COOLDOWN: &str = "DXGI capture is cooling down after a failure";

struct SharedCapture {
    capture: Option<win_capture::Capture>,
    readers: usize,
    retry: RetryCooldown,
}

// One duplication per output per process: creation, use, and the final destruction all happen
// under this mutex. The static is inert when no reader has requested pixels; it is not an owner
// that can keep capturing after every observer has stopped.
static SHARED_CAPTURE: Mutex<SharedCapture> = Mutex::new(SharedCapture {
    capture: None,
    readers: 0,
    retry: RetryCooldown::new(CAPTURE_RETRY_INTERVAL),
});

/// A lazy lease for one GameCapture. Constructing a discovery-only reader acquires nothing.
pub struct DxgiCapture {
    registered: bool,
}

impl Default for DxgiCapture {
    fn default() -> Self {
        Self::new()
    }
}

impl DxgiCapture {
    pub const fn new() -> Self {
        Self { registered: false }
    }
}

impl GameFrameSource for DxgiCapture {
    fn capture_monitor(&mut self, rect: WindowRect) -> Result<MonitorFrame, &'static str> {
        let mut shared = SHARED_CAPTURE
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if !self.registered {
            shared.readers += 1;
            self.registered = true;
        }
        // The enclosing lock admits only one retry. Healthy frames must not claim_retry():
        // that would impose a five-second interval on successful captures too.
        if !shared.retry.retry_ready() {
            return Err(ERR_COOLDOWN);
        }
        let rect = win_capture::Rect {
            x: rect.x,
            y: rect.y,
            width: rect.width,
            height: rect.height,
        };
        let result = shared
            .capture
            .get_or_insert_with(win_capture::Capture::new)
            .capture_monitor(rect);
        match result {
            Ok(frame) => {
                shared.retry.succeeded();
                Ok(MonitorFrame {
                    image: frame.image,
                    origin_x: frame.origin_x,
                    origin_y: frame.origin_y,
                    width: frame.width,
                    height: frame.height,
                })
            }
            Err(error) => {
                shared.retry.failed();
                log::warn!("[DEBUG-capture] DXGI capture failed: {error}");
                Err(error.message())
            }
        }
    }
}

impl Drop for DxgiCapture {
    fn drop(&mut self) {
        if !self.registered {
            return;
        }
        let mut shared = SHARED_CAPTURE
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        shared.readers -= 1;
        if shared.readers == 0 {
            // Drop the duplication before unlocking: a newly registered reader cannot create
            // another interface while destruction of the old one is still in progress.
            shared.capture = None;
            shared.retry.succeeded();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{DxgiCapture, ERR_COOLDOWN};
    use crate::overlay_window::WindowRect;
    use crate::reward_capture::GameFrameSource;

    // A zero-size rectangle fails before opening a device, so the ownership transitions exercise
    // the real capture error path without requiring a desktop or a GPU. This catches eager leases,
    // registering each poll again, per-reader cooldowns, and releasing another reader's cooldown.
    #[test]
    fn only_active_readers_retain_the_shared_failure_cooldown() {
        let _probe_only = DxgiCapture::new();
        let invalid = WindowRect {
            x: 0,
            y: 0,
            width: 0,
            height: 0,
        };
        let mut reward = DxgiCapture::new();
        let failure = reward
            .capture_monitor(invalid)
            .err()
            .expect("invalid rectangle");
        assert_ne!(failure, ERR_COOLDOWN);
        assert_eq!(reward.capture_monitor(invalid).err(), Some(ERR_COOLDOWN));

        let mut kiosk = DxgiCapture::new();
        assert_eq!(kiosk.capture_monitor(invalid).err(), Some(ERR_COOLDOWN));
        drop(reward);
        assert_eq!(kiosk.capture_monitor(invalid).err(), Some(ERR_COOLDOWN));
        drop(kiosk);

        // The unused probe must not keep the old failure alive after the last active reader left.
        let mut next_observation = DxgiCapture::new();
        assert_eq!(
            next_observation.capture_monitor(invalid).err(),
            Some(failure)
        );
    }
}
