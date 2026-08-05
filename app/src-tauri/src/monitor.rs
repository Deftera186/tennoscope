use warframe_acquisition::AcquisitionError;

const MARKER: &[u8] = b"Inventory sync done";
const CARRY_LIMIT: usize = 64;

pub struct LogObservation {
    identity: String,
    len: u64,
    bytes: Vec<u8>,
}
impl LogObservation {
    pub fn new(identity: impl Into<String>, len: u64, bytes: Vec<u8>) -> Self {
        Self {
            identity: identity.into(),
            len,
            bytes,
        }
    }
}

pub struct MonitorInput {
    now: u64,
    discovery: Result<Option<u32>, AcquisitionError>,
    log: Result<Option<LogObservation>, AcquisitionError>,
}
impl MonitorInput {
    pub fn running(now: u64, pid: u32, log: Option<LogObservation>) -> Self {
        Self {
            now,
            discovery: Ok(Some(pid)),
            log: Ok(log),
        }
    }
    pub fn absent(now: u64) -> Self {
        Self {
            now,
            discovery: Ok(None),
            log: Ok(None),
        }
    }
    pub fn error(now: u64, error: AcquisitionError) -> Self {
        Self {
            now,
            discovery: Err(error),
            log: Ok(None),
        }
    }
    pub fn running_with_log_error(now: u64, pid: u32) -> Self {
        Self {
            now,
            discovery: Ok(Some(pid)),
            log: Err(AcquisitionError::MemoryReadFailed { pid }),
        }
    }
}

#[derive(Debug, Eq, PartialEq)]
pub struct MonitorResult {
    pub refresh: bool,
    pub acquisition_health: Option<AcquisitionError>,
    pub log_health: Option<LogMonitorDiagnostic>,
}

pub struct MonitorMachine {
    cooldown: u64,
    process: Option<u32>,
    last_refresh: Option<u64>,
    pending: bool,
    log_identity: Option<String>,
    log_offset: u64,
    carry: Vec<u8>,
}

impl MonitorMachine {
    pub fn new(cooldown_seconds: u64) -> Self {
        Self {
            cooldown: cooldown_seconds,
            process: None,
            last_refresh: None,
            pending: false,
            log_identity: None,
            log_offset: 0,
            carry: Vec::new(),
        }
    }

    pub const fn log_offset(&self) -> u64 {
        self.log_offset
    }
    pub const fn process_pid(&self) -> Option<u32> {
        self.process
    }
    pub fn log_identity(&self) -> Option<&str> {
        self.log_identity.as_deref()
    }

    pub fn tick(&mut self, input: MonitorInput) -> MonitorResult {
        log::debug!("monitor: tick discovery={:?}", input.discovery);
        let pid = match input.discovery {
            Ok(Some(pid)) => pid,
            Ok(None) => {
                self.reset_process();
                return MonitorResult {
                    refresh: false,
                    acquisition_health: Some(AcquisitionError::GameNotRunning),
                    log_health: Some(LogMonitorDiagnostic::Unavailable),
                };
            }
            Err(error) => {
                return MonitorResult {
                    refresh: false,
                    acquisition_health: Some(error),
                    log_health: Some(LogMonitorDiagnostic::Unavailable),
                };
            }
        };
        let mut event = false;
        if self.process != Some(pid) {
            self.reset_process();
            self.process = Some(pid);
            event = true;
        }
        match input.log {
            Err(error) => {
                log::warn!("monitor: EE.log read failed: {error}");
                MonitorResult {
                    refresh: self.schedule(input.now, event),
                    acquisition_health: None,
                    log_health: Some(LogMonitorDiagnostic::ReadFailed),
                }
            }
            Ok(Some(log)) => {
                event |= self.ingest(log);
                MonitorResult {
                    refresh: self.schedule(input.now, event),
                    acquisition_health: None,
                    log_health: Some(LogMonitorDiagnostic::Ready),
                }
            }
            Ok(None) => MonitorResult {
                refresh: self.schedule(input.now, event),
                acquisition_health: None,
                log_health: Some(LogMonitorDiagnostic::Unavailable),
            },
        }
    }

    fn schedule(&mut self, now: u64, event: bool) -> bool {
        self.pending |= event;
        let ready = self
            .last_refresh
            .is_none_or(|last| now.saturating_sub(last) >= self.cooldown);
        if self.pending && ready {
            self.pending = false;
            self.last_refresh = Some(now);
            true
        } else {
            false
        }
    }

    fn ingest(&mut self, log: LogObservation) -> bool {
        let rotated =
            self.log_identity.as_deref() != Some(&log.identity) || log.len < self.log_offset;
        if rotated {
            log::info!("monitor: EE.log rotated or changed identity");
            self.log_identity = Some(log.identity);
            self.log_offset = 0;
            self.carry.clear();
        }
        if self.log_offset == 0 && log.bytes.is_empty() {
            self.log_offset = log.len;
            return false;
        }
        let mut joined = std::mem::take(&mut self.carry);
        joined.extend_from_slice(&log.bytes);
        let mut triggered = false;
        let mut line_start = 0;
        for (index, byte) in joined.iter().enumerate() {
            if *byte == b'\n' {
                triggered |= joined[line_start..index]
                    .windows(MARKER.len())
                    .any(|window| window == MARKER);
                line_start = index + 1;
            }
        }
        self.carry.extend_from_slice(&joined[line_start..]);
        if self.carry.len() > CARRY_LIMIT {
            self.carry.drain(..self.carry.len() - CARRY_LIMIT);
        }
        self.log_offset = log.len;
        triggered
    }

    fn reset_process(&mut self) {
        log::info!("monitor: game process gone, resetting");
        self.process = None;
        self.log_identity = None;
        self.log_offset = 0;
        self.carry.clear();
        self.pending = false;
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LogMonitorDiagnostic {
    Ready,
    Unavailable,
    ReadFailed,
}
