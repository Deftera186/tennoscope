use std::{
    cmp::Reverse,
    collections::HashMap,
    fs::{self, File},
    io,
    os::unix::fs::FileExt,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use crate::{
    AcquisitionError, GameProcess, MemoryReader, ProcessDiscovery, ReadableRegion,
    RegionScanPriority,
};

const FULL_PROCESS_NAME: &str = "Warframe.x64.exe";
const WINE_PROCESS_NAME: &str = "Warframe.x64.ex";

/// Linux and Proton process access backed by procfs.
///
/// The alternate root constructor exists so tests can exercise discovery and
/// memory behavior without touching a real process.
pub struct LinuxProc {
    root: PathBuf,
    memory_files: Mutex<HashMap<GameProcess, Arc<File>>>,
}

impl LinuxProc {
    pub fn new() -> Self {
        Self::at("/proc")
    }

    pub fn at(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            memory_files: Mutex::new(HashMap::new()),
        }
    }

    fn process_file(&self, pid: u32, name: &str) -> PathBuf {
        self.root.join(pid.to_string()).join(name)
    }

    fn start_time(&self, pid: u32) -> Result<u64, AcquisitionError> {
        let stat = fs::read_to_string(self.process_file(pid, "stat"))
            .map_err(|error| classify_io(pid, error))?;
        parse_start_time(&stat).ok_or(AcquisitionError::ProcessDiscoveryFailed)
    }

    fn validate_identity(&self, process: &GameProcess) -> Result<(), AcquisitionError> {
        let Some(expected) = process.start_time_ticks() else {
            return Ok(());
        };
        match self.start_time(process.pid()) {
            Ok(actual) if actual == expected => Ok(()),
            Ok(_) | Err(AcquisitionError::ProcessExited { .. }) => {
                Err(AcquisitionError::ProcessExited { pid: process.pid() })
            }
            Err(error) => Err(error),
        }
    }

    fn memory_file(&self, process: &GameProcess) -> Result<Arc<File>, AcquisitionError> {
        if process.start_time_ticks().is_none() {
            self.validate_identity(process)?;
            return File::open(self.process_file(process.pid(), "mem"))
                .map(Arc::new)
                .map_err(|error| classify_io(process.pid(), error));
        }
        let mut files = self
            .memory_files
            .lock()
            .map_err(|_| AcquisitionError::MemoryReadFailed { pid: process.pid() })?;
        if let Some(file) = files.get(process) {
            return Ok(Arc::clone(file));
        }

        self.validate_identity(process)?;
        let file = Arc::new(
            File::open(self.process_file(process.pid(), "mem"))
                .map_err(|error| classify_io(process.pid(), error))?,
        );
        self.validate_identity(process)?;
        files.insert(*process, Arc::clone(&file));
        Ok(file)
    }
}

impl Default for LinuxProc {
    fn default() -> Self {
        Self::new()
    }
}

impl ProcessDiscovery for LinuxProc {
    fn discover(&self) -> Result<Option<GameProcess>, AcquisitionError> {
        let entries =
            fs::read_dir(&self.root).map_err(|_| AcquisitionError::ProcessDiscoveryFailed)?;
        let mut candidates = Vec::new();

        for entry in entries.flatten() {
            let Some(pid) = entry
                .file_name()
                .to_str()
                .and_then(|name| name.parse::<u32>().ok())
            else {
                continue;
            };
            let Ok(comm) = fs::read_to_string(self.process_file(pid, "comm")) else {
                continue;
            };
            let comm = comm.trim_end();
            let priority = match comm {
                FULL_PROCESS_NAME => 2_u8,
                WINE_PROCESS_NAME => 1_u8,
                _ => continue,
            };
            let start_time = match self.start_time(pid) {
                Ok(start_time) => start_time,
                Err(AcquisitionError::ProcessExited { .. }) => continue,
                Err(AcquisitionError::MemoryPermissionDenied { .. }) => {
                    return Err(AcquisitionError::MemoryPermissionDenied { pid });
                }
                Err(_) => return Err(AcquisitionError::ProcessDiscoveryFailed),
            };
            let maps = match fs::read_to_string(self.process_file(pid, "maps")) {
                Ok(maps) => maps,
                Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                Err(error) if error.kind() == io::ErrorKind::PermissionDenied => {
                    return Err(AcquisitionError::MemoryPermissionDenied { pid });
                }
                Err(_) => return Err(AcquisitionError::ProcessDiscoveryFailed),
            };
            let confirmed_start_time = match self.start_time(pid) {
                Ok(start_time) => start_time,
                Err(AcquisitionError::ProcessExited { .. }) => continue,
                Err(AcquisitionError::MemoryPermissionDenied { .. }) => {
                    return Err(AcquisitionError::MemoryPermissionDenied { pid });
                }
                Err(_) => return Err(AcquisitionError::ProcessDiscoveryFailed),
            };
            if start_time == confirmed_start_time && maps.lines().any(maps_game_executable) {
                candidates.push((priority, pid, start_time));
            }
        }

        candidates.sort_unstable_by_key(|&(priority, pid, _)| (Reverse(priority), pid));
        let selected = candidates
            .first()
            .map(|&(_, pid, start_time)| GameProcess::identified(pid, start_time));
        self.memory_files
            .lock()
            .map_err(|_| AcquisitionError::ProcessDiscoveryFailed)?
            .retain(|process, _| Some(*process) == selected);
        Ok(selected)
    }
}

impl MemoryReader for LinuxProc {
    fn readable_regions(
        &self,
        process: &GameProcess,
    ) -> Result<Vec<ReadableRegion>, AcquisitionError> {
        self.validate_identity(process)?;
        let path = self.process_file(process.pid(), "maps");
        let maps = fs::read_to_string(path).map_err(|error| classify_io(process.pid(), error))?;
        self.validate_identity(process)?;
        Ok(maps.lines().filter_map(parse_readable_region).collect())
    }

    fn read_at(
        &self,
        process: &GameProcess,
        address: u64,
        buffer: &mut [u8],
    ) -> Result<usize, AcquisitionError> {
        if buffer.is_empty() {
            self.validate_identity(process)?;
            return Ok(0);
        }
        let file = self.memory_file(process)?;
        match file.read_at(buffer, address) {
            Ok(read) => Ok(read),
            Err(error) if error.raw_os_error() == Some(libc::EIO) => {
                self.validate_identity(process)?;
                // Linux reports EIO for readable-looking mappings that cannot
                // actually be read through procfs. Treat only that mapping as
                // unavailable so the scanner can continue with the next one.
                Ok(0)
            }
            Err(error) => Err(classify_io(process.pid(), error)),
        }
    }
}

fn parse_start_time(stat: &str) -> Option<u64> {
    stat.rsplit_once(") ")?
        .1
        .split_ascii_whitespace()
        .nth(19)?
        .parse()
        .ok()
}

fn maps_game_executable(line: &str) -> bool {
    let Some(path) = line.split_ascii_whitespace().skip(5).last() else {
        return false;
    };
    Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name == FULL_PROCESS_NAME)
}

fn parse_readable_region(line: &str) -> Option<ReadableRegion> {
    let mut fields = line.split_ascii_whitespace();
    let range = fields.next()?;
    let permissions = fields.next()?;
    if !permissions.starts_with('r') {
        return None;
    }
    let path = line.split_ascii_whitespace().nth(5);
    if path.is_some_and(|path| matches!(path, "[vdso]" | "[vvar]" | "[vsyscall]")) {
        return None;
    }
    let (start, end) = range.split_once('-')?;
    let start = u64::from_str_radix(start, 16).ok()?;
    let end = u64::from_str_radix(end, 16).ok()?;
    let len = usize::try_from(end.checked_sub(start)?).ok()?;
    let file_backed = path.is_some_and(|path| !path.starts_with('['));
    let scan_priority = if file_backed {
        RegionScanPriority::FileBacked
    } else if permissions.as_bytes().get(1) == Some(&b'w') {
        RegionScanPriority::WritableAnonymous
    } else {
        RegionScanPriority::Anonymous
    };
    (len != 0).then_some(ReadableRegion::classified(start, len, scan_priority))
}

fn classify_io(pid: u32, error: io::Error) -> AcquisitionError {
    match error.kind() {
        io::ErrorKind::NotFound => AcquisitionError::ProcessExited { pid },
        io::ErrorKind::PermissionDenied => AcquisitionError::MemoryPermissionDenied { pid },
        _ => AcquisitionError::MemoryReadFailed { pid },
    }
}
