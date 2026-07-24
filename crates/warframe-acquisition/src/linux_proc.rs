use std::{
    cmp::Reverse,
    fs::{self, File},
    io,
    os::unix::fs::FileExt,
    path::{Path, PathBuf},
};

use crate::{AcquisitionError, GameProcess, MemoryReader, ProcessDiscovery, ReadableRegion};

const FULL_PROCESS_NAME: &str = "Warframe.x64.exe";
const WINE_PROCESS_NAME: &str = "Warframe.x64.ex";

/// Linux and Proton process access backed by procfs.
///
/// The alternate root constructor exists so tests can exercise discovery and
/// memory behavior without touching a real process.
pub struct LinuxProc {
    root: PathBuf,
}

impl LinuxProc {
    pub fn new() -> Self {
        Self::at("/proc")
    }

    pub fn at(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    fn process_file(&self, pid: u32, name: &str) -> PathBuf {
        self.root.join(pid.to_string()).join(name)
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
            let Ok(maps) = fs::read_to_string(self.process_file(pid, "maps")) else {
                continue;
            };
            if maps.lines().any(maps_game_executable) {
                candidates.push((priority, pid));
            }
        }

        candidates.sort_unstable_by_key(|&(priority, pid)| (Reverse(priority), pid));
        Ok(candidates.first().map(|&(_, pid)| GameProcess::new(pid)))
    }
}

impl MemoryReader for LinuxProc {
    fn readable_regions(
        &self,
        process: &GameProcess,
    ) -> Result<Vec<ReadableRegion>, AcquisitionError> {
        let path = self.process_file(process.pid(), "maps");
        let maps = fs::read_to_string(path).map_err(|error| classify_io(process.pid(), error))?;
        Ok(maps.lines().filter_map(parse_readable_region).collect())
    }

    fn read_at(
        &self,
        process: &GameProcess,
        address: u64,
        buffer: &mut [u8],
    ) -> Result<usize, AcquisitionError> {
        if buffer.is_empty() {
            return Ok(0);
        }
        let path = self.process_file(process.pid(), "mem");
        let file = File::open(path).map_err(|error| classify_io(process.pid(), error))?;
        match file.read_at(buffer, address) {
            Ok(read) => Ok(read),
            Err(error) if error.raw_os_error() == Some(libc::EIO) => {
                if self.root.join(process.pid().to_string()).exists() {
                    // Linux reports EIO for readable-looking mappings that cannot
                    // actually be read through procfs. Treat only that mapping as
                    // unavailable so the scanner can continue with the next one.
                    Ok(0)
                } else {
                    Err(AcquisitionError::ProcessExited { pid: process.pid() })
                }
            }
            Err(error) => Err(classify_io(process.pid(), error)),
        }
    }
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
    (len != 0).then_some(ReadableRegion::new(start, len))
}

fn classify_io(pid: u32, error: io::Error) -> AcquisitionError {
    match error.kind() {
        io::ErrorKind::NotFound => AcquisitionError::ProcessExited { pid },
        io::ErrorKind::PermissionDenied => AcquisitionError::MemoryPermissionDenied { pid },
        _ => AcquisitionError::MemoryReadFailed { pid },
    }
}
