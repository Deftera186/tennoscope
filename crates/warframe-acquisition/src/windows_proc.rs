use std::{
    cmp::Reverse,
    collections::HashMap,
    ffi::OsStr,
    io,
    sync::{Arc, Mutex},
};

use proc_maps::get_process_maps;
use read_process_memory::{CopyAddress, ProcessHandle};
use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System};

use crate::{
    AcquisitionError, GameProcess, MemoryReader, ProcessDiscovery, ReadableRegion,
    RegionScanPriority,
};

/// Native Windows builds run under this exact image name. The Linux backend also matches a
/// 15-character truncation because that is what `/proc/<pid>/comm` stores; Windows has no such
/// limit, so carrying that constant over would only invite a false positive.
const PROCESS_NAME: &str = "Warframe.x64.exe";

const PAGE_SIZE: usize = 4096;

/// `ReadProcessMemory` fails the whole call rather than reporting how far it got, and returns one
/// of these when the failure is "this page is not readable" rather than "this process is gone".
const ERROR_ACCESS_DENIED: i32 = 5;
const ERROR_INVALID_HANDLE: i32 = 6;
/// `OpenProcess` against a PID that no live process owns -- which is what a game that has just
/// quit looks like -- reports this rather than a not-found of any kind.
const ERROR_INVALID_PARAMETER: i32 = 87;
const ERROR_NOACCESS: i32 = 998;
const ERROR_PARTIAL_COPY: i32 = 299;

/// Windows process access backed by `ReadProcessMemory` and `VirtualQueryEx`.
///
/// Two differences from [`crate::LinuxProc`] are worth stating outright.
///
/// There is no soft-dirty equivalent: `GetWriteWatch` needs `MEM_WRITE_WATCH` at allocation time
/// and only works on the calling process, so this backend falls back to the `MemoryReader`
/// defaults. Every poll therefore rescans every region rather than only the pages the game wrote.
///
/// PID reuse is guarded by the cached handle rather than by a start time comparison: a live
/// `PROCESS_VM_READ` handle pins the PID for as long as it is held, so the kernel cannot hand that
/// number to another process behind our back. `GameProcess::start_time_ticks` still carries
/// `sysinfo`'s value, but that is seconds-since-epoch here and clock-ticks-since-boot on Linux --
/// the two must never be compared across backends.
pub struct WindowsProc {
    handles: Mutex<HashMap<GameProcess, Arc<ProcessHandle>>>,
}

impl WindowsProc {
    pub fn new() -> Self {
        Self {
            handles: Mutex::new(HashMap::new()),
        }
    }

    fn handle(&self, process: &GameProcess) -> Result<Arc<ProcessHandle>, AcquisitionError> {
        let mut handles = self
            .handles
            .lock()
            .map_err(|_| AcquisitionError::MemoryReadFailed { pid: process.pid() })?;
        if let Some(handle) = handles.get(process) {
            return Ok(Arc::clone(handle));
        }
        // `OpenProcess` is also the elevation probe: `proc-maps` 0.5.0 compares its own handle
        // against `INVALID_HANDLE_VALUE` when the failure returns NULL, so asking it instead would
        // report `ERROR_INVALID_HANDLE` for what is really an access denial.
        let handle = Arc::new(
            ProcessHandle::try_from(process.pid())
                .map_err(|error| classify_io(process.pid(), &error))?,
        );
        handles.insert(*process, Arc::clone(&handle));
        Ok(handle)
    }
}

impl Default for WindowsProc {
    fn default() -> Self {
        Self::new()
    }
}

impl ProcessDiscovery for WindowsProc {
    fn discover(&self) -> Result<Option<GameProcess>, AcquisitionError> {
        let mut system = System::new();
        system.refresh_processes_specifics(
            ProcessesToUpdate::All,
            true,
            ProcessRefreshKind::nothing(),
        );
        let mut candidates = system
            .processes_by_exact_name(OsStr::new(PROCESS_NAME))
            .map(|process| (process.pid().as_u32(), process.start_time()))
            .collect::<Vec<_>>();

        // Oldest first, then lowest PID: a launcher relaunch can leave the dying instance visible
        // for a moment, and the surviving one is the one that has been up longest.
        candidates.sort_unstable_by_key(|&(pid, start_time)| (Reverse(start_time), pid));
        let selected = candidates
            .first()
            .map(|&(pid, start_time)| GameProcess::identified(pid, start_time));
        self.handles
            .lock()
            .map_err(|_| AcquisitionError::ProcessDiscoveryFailed)?
            .retain(|process, _| Some(*process) == selected);
        Ok(selected)
    }
}

impl MemoryReader for WindowsProc {
    fn readable_regions(
        &self,
        process: &GameProcess,
    ) -> Result<Vec<ReadableRegion>, AcquisitionError> {
        // Holding the handle across the enumeration is what keeps the PID from being reused
        // underneath it; the addresses are meaningless if the process behind them changed.
        let _handle = self.handle(process)?;
        let maps =
            get_process_maps(process.pid()).map_err(|error| classify_io(process.pid(), &error))?;
        Ok(maps
            .iter()
            .filter_map(|map| {
                readable_region(
                    map.start(),
                    map.size(),
                    map.is_read(),
                    map.is_write(),
                    map.filename().is_some(),
                )
            })
            .collect())
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
        let handle = self.handle(process)?;
        let Ok(address) = usize::try_from(address) else {
            return Ok(0);
        };
        read_chunked(handle.as_ref(), address, buffer).map_err(|error| {
            let mut classified = classify_io(process.pid(), &error);
            // A dead process reports the same denial as a guard page; only the handle knows which,
            // and it stays valid after exit. Downgrade nothing here -- the scanner treats a denial
            // as fatal, which is right when the very first page of the very first region fails.
            if matches!(classified, AcquisitionError::MemoryReadFailed { .. }) {
                classified = AcquisitionError::MemoryReadFailed { pid: process.pid() };
            }
            classified
        })
    }
}

/// Read as much of `buffer` as the process will give us, page by page.
///
/// `copy_address` returns `io::Result<()>`, not a byte count -- it passes `lpNumberOfBytesRead` as
/// NULL -- so a straddling read that covers one bad page fails entirely and tells us nothing about
/// where. Retrying at page granularity recovers the Linux backend's short-read behaviour: stop at
/// the first page the process refuses and report how much came before it. `PAGE_GUARD` regions land
/// here too; `proc-maps` neither masks `PAGE_GUARD` off `Protect` nor exposes it, so tolerating the
/// failure is the only option available.
fn read_chunked(source: &impl CopyAddress, address: usize, buffer: &mut [u8]) -> io::Result<usize> {
    if source.copy_address(address, buffer).is_ok() {
        return Ok(buffer.len());
    }

    let mut done = 0;
    while done < buffer.len() {
        // First chunk runs to the next page boundary so every later chunk is page aligned.
        let boundary = PAGE_SIZE - ((address + done) % PAGE_SIZE);
        let end = (done + boundary).min(buffer.len());
        match source.copy_address(address + done, &mut buffer[done..end]) {
            Ok(()) => done = end,
            Err(error) if is_unreadable_page(&error) => return Ok(done),
            Err(error) if done != 0 => {
                let _ = error;
                return Ok(done);
            }
            Err(error) => return Err(error),
        }
    }
    Ok(done)
}

fn is_unreadable_page(error: &io::Error) -> bool {
    matches!(
        error.raw_os_error(),
        Some(ERROR_ACCESS_DENIED | ERROR_NOACCESS | ERROR_PARTIAL_COPY)
    )
}

fn readable_region(
    start: usize,
    size: usize,
    read: bool,
    write: bool,
    file_backed: bool,
) -> Option<ReadableRegion> {
    if !read || size == 0 {
        return None;
    }
    let scan_priority = match (file_backed, write) {
        (true, true) => RegionScanPriority::WritablePrivateFileBacked,
        (true, false) => RegionScanPriority::FileBacked,
        (false, true) => RegionScanPriority::WritableAnonymous,
        (false, false) => RegionScanPriority::Anonymous,
    };
    Some(ReadableRegion::classified(
        start as u64,
        size,
        scan_priority,
    ))
}

fn classify_io(pid: u32, error: &io::Error) -> AcquisitionError {
    match error.raw_os_error() {
        Some(ERROR_ACCESS_DENIED) => AcquisitionError::MemoryPermissionDenied { pid },
        // `OpenProcess` against a PID nobody owns is how an exited process presents itself.
        Some(ERROR_INVALID_HANDLE | ERROR_INVALID_PARAMETER) => {
            AcquisitionError::ProcessExited { pid }
        }
        _ => AcquisitionError::MemoryReadFailed { pid },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    /// A process whose pages past `readable_until` refuse every read, so the chunking path is
    /// exercised without a live process.
    struct FakeProcess {
        readable_until: usize,
        error: i32,
        calls: RefCell<Vec<(usize, usize)>>,
    }

    impl CopyAddress for FakeProcess {
        fn copy_address(&self, addr: usize, buf: &mut [u8]) -> io::Result<()> {
            self.calls.borrow_mut().push((addr, buf.len()));
            if addr + buf.len() > self.readable_until {
                return Err(io::Error::from_raw_os_error(self.error));
            }
            buf.fill(0xAB);
            Ok(())
        }
    }

    fn fake(readable_until: usize, error: i32) -> FakeProcess {
        FakeProcess {
            readable_until,
            error,
            calls: RefCell::new(Vec::new()),
        }
    }

    #[test]
    fn a_fully_readable_span_costs_exactly_one_call() {
        let source = fake(usize::MAX, ERROR_PARTIAL_COPY);
        let mut buffer = [0_u8; 8192];
        assert_eq!(read_chunked(&source, 0x1000, &mut buffer).unwrap(), 8192);
        assert_eq!(source.calls.borrow().len(), 1, "no per-page retry needed");
        assert!(buffer.iter().all(|byte| *byte == 0xAB));
    }

    #[test]
    fn a_span_ending_in_an_unreadable_page_reports_the_readable_prefix() {
        // Readable through 0x3000; the read starts mid-page so the first chunk is a partial one.
        let source = fake(0x3000, ERROR_PARTIAL_COPY);
        let mut buffer = [0_u8; 0x2800];
        let read = read_chunked(&source, 0x1800, &mut buffer).unwrap();
        assert_eq!(read, 0x1800, "should stop at the first refused page");
        assert!(buffer[..read].iter().all(|byte| *byte == 0xAB));
        assert!(
            buffer[read..].iter().all(|byte| *byte == 0),
            "bytes past the refusal must be left untouched"
        );
        // 0x1800 -> 0x2000 (partial), 0x2000 -> 0x3000, then the refusal.
        assert_eq!(
            &source.calls.borrow()[1..],
            &[(0x1800, 0x800), (0x2000, 0x1000), (0x3000, 0x1000)]
        );
    }

    #[test]
    fn every_unreadable_page_error_is_tolerated_rather_than_raised() {
        for error in [ERROR_ACCESS_DENIED, ERROR_NOACCESS, ERROR_PARTIAL_COPY] {
            let source = fake(0, error);
            let mut buffer = [0_u8; 4096];
            assert_eq!(
                read_chunked(&source, 0x1000, &mut buffer).unwrap(),
                0,
                "error {error} should read as an unavailable page, not a failure"
            );
        }
    }

    #[test]
    fn an_unexpected_error_on_the_very_first_page_is_raised() {
        let source = fake(0, ERROR_INVALID_HANDLE);
        let mut buffer = [0_u8; 4096];
        let error = read_chunked(&source, 0x1000, &mut buffer).unwrap_err();
        assert_eq!(error.raw_os_error(), Some(ERROR_INVALID_HANDLE));
    }

    #[test]
    fn regions_are_classified_the_way_the_procfs_backend_classifies_them() {
        assert_eq!(
            readable_region(0x1000, 0x2000, true, true, false)
                .unwrap()
                .scan_priority(),
            RegionScanPriority::WritableAnonymous
        );
        assert_eq!(
            readable_region(0x1000, 0x2000, true, true, true)
                .unwrap()
                .scan_priority(),
            RegionScanPriority::WritablePrivateFileBacked
        );
        assert_eq!(
            readable_region(0x1000, 0x2000, true, false, true)
                .unwrap()
                .scan_priority(),
            RegionScanPriority::FileBacked
        );
        assert_eq!(
            readable_region(0x1000, 0x2000, true, false, false)
                .unwrap()
                .scan_priority(),
            RegionScanPriority::Anonymous
        );
        assert!(
            readable_region(0x1000, 0x2000, false, true, false).is_none(),
            "an unreadable region is not worth handing to the scanner"
        );
        assert!(readable_region(0x1000, 0, true, true, false).is_none());
    }

    #[test]
    fn open_process_failures_name_the_condition_the_caller_can_act_on() {
        let denied = io::Error::from_raw_os_error(ERROR_ACCESS_DENIED);
        assert_eq!(
            classify_io(7, &denied),
            AcquisitionError::MemoryPermissionDenied { pid: 7 }
        );
        for gone in [ERROR_INVALID_HANDLE, ERROR_INVALID_PARAMETER] {
            assert_eq!(
                classify_io(7, &io::Error::from_raw_os_error(gone)),
                AcquisitionError::ProcessExited { pid: 7 },
                "error {gone} means there is no process to read"
            );
        }
    }
}
