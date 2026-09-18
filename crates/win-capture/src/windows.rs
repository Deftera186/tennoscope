//! Serialized DXGI desktop duplication. Only this Windows-only module crosses FFI.
//!
//! The application owns serialization and retry policy. No immediate-context call
//! or acquired/mapped resource escapes a capture call, including during unwinding.

use std::{
    error::Error,
    fmt,
    time::{Duration, Instant},
};

use ::windows::{
    Win32::{
        Foundation::{HMODULE, RECT},
        Graphics::{
            Direct3D::D3D_DRIVER_TYPE_UNKNOWN,
            Direct3D11::{
                D3D11_CPU_ACCESS_READ, D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                D3D11_MAP_FLAG_DO_NOT_WAIT, D3D11_MAP_READ, D3D11_MAPPED_SUBRESOURCE,
                D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC, D3D11_USAGE_STAGING, D3D11CreateDevice,
                ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D,
            },
            Dxgi::{
                Common::{
                    DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_MODE_ROTATION, DXGI_MODE_ROTATION_IDENTITY,
                    DXGI_MODE_ROTATION_ROTATE90, DXGI_MODE_ROTATION_ROTATE180,
                    DXGI_MODE_ROTATION_ROTATE270, DXGI_MODE_ROTATION_UNSPECIFIED, DXGI_SAMPLE_DESC,
                },
                CreateDXGIFactory1, DXGI_ERROR_NOT_FOUND, DXGI_ERROR_WAIT_TIMEOUT,
                DXGI_ERROR_WAS_STILL_DRAWING, DXGI_OUTDUPL_FRAME_INFO, DXGI_OUTPUT_DESC,
                IDXGIAdapter1, IDXGIFactory1, IDXGIOutput1, IDXGIOutputDuplication, IDXGIResource,
            },
            Gdi::{MONITOR_DEFAULTTONULL, MonitorFromRect},
        },
        UI::HiDpi::{
            DPI_AWARENESS_CONTEXT, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE,
            SetThreadDpiAwarenessContext,
        },
    },
    core::{HRESULT, Interface},
};

use crate::{
    CapturedFrame, Rect,
    mapped::bgra_rows,
    pixels::{Rotation, decode_bgra},
};

const ACQUIRE_TIMEOUT_MS: u32 = 100;

/// A capture failure with a stable app-facing message and native diagnostic context.
#[derive(Debug)]
pub struct CaptureError {
    message: &'static str,
    operation: &'static str,
    hresult: Option<HRESULT>,
}

impl CaptureError {
    pub fn message(&self) -> &'static str {
        self.message
    }

    fn invalid(operation: &'static str, message: &'static str) -> Self {
        Self {
            message,
            operation,
            hresult: None,
        }
    }

    fn native(
        operation: &'static str,
        message: &'static str,
        error: ::windows::core::Error,
    ) -> Self {
        Self {
            message,
            operation,
            hresult: Some(error.code()),
        }
    }

    fn is_transient(&self) -> bool {
        (self.operation == "IDXGIOutputDuplication::AcquireNextFrame"
            && self.hresult == Some(DXGI_ERROR_WAIT_TIMEOUT))
            || (self.operation == "ID3D11DeviceContext::Map"
                && self.hresult == Some(DXGI_ERROR_WAS_STILL_DRAWING))
    }
}

impl fmt::Display for CaptureError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.message, self.operation)?;
        if let Some(code) = self.hresult {
            write!(formatter, " (HRESULT 0x{:08X})", code.0 as u32)?;
        }
        Ok(())
    }
}

impl Error for CaptureError {}

/// A reusable capture session. Calls require exclusive access, but may move threads.
///
/// The COM interfaces are Send in windows-rs. We never retain raw monitor handles
/// or mapped pointers, and do not create a SINGLETHREADED D3D11 device.
#[derive(Default)]
pub struct Capture {
    session: Option<Session>,
}

impl Capture {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn capture_monitor(&mut self, rect: Rect) -> Result<CapturedFrame, CaptureError> {
        // Until this call succeeds (or merely times out), the session is local.
        // Errors and unwinding therefore invalidate it, including failed releases.
        let previous = self.session.take();
        let rect = physical_rect(rect)?;
        let _dpi = DpiScope::enter()?;
        // SAFETY: rect is a checked, nonempty RECT and remains live for the call.
        // The current thread uses physical per-monitor-aware coordinates.
        let monitor = unsafe { MonitorFromRect(&rect, MONITOR_DEFAULTTONULL) };
        if monitor.0.is_null() {
            return Err(CaptureError::invalid(
                "MonitorFromRect",
                "The game window does not intersect a display",
            ));
        }
        let monitor = monitor.0 as usize;
        let mut session = match previous {
            Some(session) if session.matches(monitor)? => session,
            previous => {
                // One duplication interface per output per process: destroy the
                // old interface before creating its replacement, not afterward.
                drop(previous);
                Session::new(monitor)?
            }
        };
        let result = session.capture();
        if result.is_ok() || result.as_ref().is_err_and(|error| error.is_transient()) {
            self.session = Some(session);
        }
        result
    }
}

fn physical_rect(rect: Rect) -> Result<RECT, CaptureError> {
    let invalid = || CaptureError::invalid("capture rectangle", "Invalid game window bounds");
    if rect.width == 0 || rect.height == 0 {
        return Err(invalid());
    }
    // Use i64 so even a u32-sized rectangle spanning negative coordinates is
    // accepted when both endpoints are representable as Win32 LONGs.
    let right = i32::try_from(i64::from(rect.x) + i64::from(rect.width)).map_err(|_| invalid())?;
    let bottom =
        i32::try_from(i64::from(rect.y) + i64::from(rect.height)).map_err(|_| invalid())?;
    Ok(RECT {
        left: rect.x,
        top: rect.y,
        right,
        bottom,
    })
}

/// Thread-affine by construction: the raw context is not Send and never stored
/// in Capture. Restores even when capture returns early or unwinds.
struct DpiScope(DPI_AWARENESS_CONTEXT);

impl DpiScope {
    fn enter() -> Result<Self, CaptureError> {
        // SAFETY: the predefined context is valid on the supported Windows 10+
        // API, and only changes this thread. The returned context is kept local.
        let previous =
            unsafe { SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE) };
        // Do not use is_invalid(): -1 is the valid DPI-unaware context.
        if previous.0.is_null() {
            return Err(CaptureError::invalid(
                "SetThreadDpiAwarenessContext",
                "Could not select physical display coordinates",
            ));
        }
        Ok(Self(previous))
    }
}

impl Drop for DpiScope {
    fn drop(&mut self) {
        // SAFETY: this is the context returned by the successful override on
        // this same thread; the guard cannot move to another thread.
        unsafe { SetThreadDpiAwarenessContext(self.0) };
    }
}

/// Snapshot the descriptor without retaining its non-Send raw HMONITOR. The
/// integer is only an opaque comparison key, never dereferenced or reconstructed.
struct OutputDescription {
    monitor: usize,
    device_name: [u16; 32],
    bounds: RECT,
    rotation: DXGI_MODE_ROTATION,
}

impl OutputDescription {
    fn new(desc: DXGI_OUTPUT_DESC) -> Self {
        Self {
            monitor: desc.Monitor.0 as usize,
            device_name: desc.DeviceName,
            bounds: desc.DesktopCoordinates,
            rotation: desc.Rotation,
        }
    }

    fn matches(&self, desc: &DXGI_OUTPUT_DESC) -> bool {
        desc.AttachedToDesktop.as_bool()
            && self.monitor == desc.Monitor.0 as usize
            && self.device_name == desc.DeviceName
            && self.bounds == desc.DesktopCoordinates
            && self.rotation == desc.Rotation
    }

    fn rotation(&self) -> Result<Rotation, CaptureError> {
        match self.rotation {
            // Microsoft's desktop duplication sample treats unspecified as identity.
            DXGI_MODE_ROTATION_UNSPECIFIED | DXGI_MODE_ROTATION_IDENTITY => Ok(Rotation::Identity),
            DXGI_MODE_ROTATION_ROTATE90 => Ok(Rotation::Clockwise90),
            DXGI_MODE_ROTATION_ROTATE180 => Ok(Rotation::Clockwise180),
            DXGI_MODE_ROTATION_ROTATE270 => Ok(Rotation::Clockwise270),
            _ => Err(CaptureError::invalid(
                "DXGI output rotation",
                "Unsupported display rotation",
            )),
        }
    }

    fn validate_texture(&self, texture: &D3D11_TEXTURE2D_DESC) -> Result<(), CaptureError> {
        if texture.Format != DXGI_FORMAT_B8G8R8A8_UNORM {
            return Err(CaptureError::invalid(
                "ID3D11Texture2D::GetDesc",
                "Unsupported desktop pixel format",
            ));
        }
        if texture.Width == 0
            || texture.Height == 0
            || texture.MipLevels != 1
            || texture.ArraySize != 1
            || texture.SampleDesc.Count != 1
            || texture.SampleDesc.Quality != 0
        {
            return Err(CaptureError::invalid(
                "ID3D11Texture2D::GetDesc",
                "Invalid desktop texture layout",
            ));
        }
        let (width, height) = match self.rotation()? {
            Rotation::Identity | Rotation::Clockwise180 => (texture.Width, texture.Height),
            Rotation::Clockwise90 | Rotation::Clockwise270 => (texture.Height, texture.Width),
        };
        let desktop_width = i64::from(self.bounds.right) - i64::from(self.bounds.left);
        let desktop_height = i64::from(self.bounds.bottom) - i64::from(self.bounds.top);
        if i64::from(width) != desktop_width || i64::from(height) != desktop_height {
            return Err(CaptureError::invalid(
                "DXGI desktop geometry",
                "Desktop texture does not match display bounds",
            ));
        }
        Ok(())
    }
}

struct Session {
    // Drop the duplication before its supporting interfaces.
    duplication: IDXGIOutputDuplication,
    // Retained only after a full CopyResource was submitted. Map must succeed
    // before any pixels are returned, including a copy still pending on the GPU.
    staging: Option<Staging>,
    context: ID3D11DeviceContext,
    device: ID3D11Device,
    output: IDXGIOutput1,
    factory: IDXGIFactory1,
    description: OutputDescription,
}

impl Session {
    fn new(monitor: usize) -> Result<Self, CaptureError> {
        // SAFETY: windows-rs supplies the matching IID and owns the returned COM reference.
        let factory: IDXGIFactory1 = unsafe { CreateDXGIFactory1() }.map_err(|error| {
            CaptureError::native("CreateDXGIFactory1", "Could not enumerate displays", error)
        })?;
        let (adapter, output, description) = find_output(&factory, monitor)?;
        description.rotation()?;
        let mut device = None;
        let mut context = None;
        // SAFETY: adapter comes from this factory and owns the selected output.
        // UNKNOWN with a non-null adapter and null software module is required.
        // Both out-pointers are valid, and no SINGLETHREADED flag is requested.
        unsafe {
            D3D11CreateDevice(
                &adapter,
                D3D_DRIVER_TYPE_UNKNOWN,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                None,
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                Some(&mut context),
            )
        }
        .map_err(|error| {
            CaptureError::native(
                "D3D11CreateDevice",
                "Could not create display capture device",
                error,
            )
        })?;
        let device = device.ok_or_else(|| {
            CaptureError::invalid("D3D11CreateDevice", "No capture device was returned")
        })?;
        let context = context.ok_or_else(|| {
            CaptureError::invalid("D3D11CreateDevice", "No capture context was returned")
        })?;
        // SAFETY: device was created on this output's adapter from IDXGIFactory1.
        // Capture dropped its previous duplication before entering this constructor.
        let duplication = unsafe { output.DuplicateOutput(&device) }.map_err(|error| {
            CaptureError::native(
                "IDXGIOutput1::DuplicateOutput",
                "Could not start desktop duplication",
                error,
            )
        })?;
        Ok(Self {
            duplication,
            staging: None,
            context,
            device,
            output,
            factory,
            description,
        })
    }

    fn matches(&self, monitor: usize) -> Result<bool, CaptureError> {
        // SAFETY: factory is a live owned interface; IsCurrent has no pointer inputs.
        if self.description.monitor != monitor || !unsafe { self.factory.IsCurrent() }.as_bool() {
            return Ok(false);
        }
        // SAFETY: output is live; windows-rs owns the descriptor out-parameter.
        let desc = unsafe { self.output.GetDesc() }.map_err(|error| {
            CaptureError::native(
                "IDXGIOutput::GetDesc",
                "Could not query the selected display",
                error,
            )
        })?;
        Ok(self.description.matches(&desc))
    }

    fn capture(&mut self) -> Result<CapturedFrame, CaptureError> {
        // Bounds our acquire/readback waiting, not time inside driver entrypoints
        // or CPU pixel conversion. There is no unbounded GPU readback wait.
        let deadline = Instant::now() + Duration::from_millis(u64::from(ACQUIRE_TIMEOUT_MS));
        let mut info = DXGI_OUTDUPL_FRAME_INFO::default();
        let mut resource = None;
        // SAFETY: exclusive capture access serializes this duplication/context.
        // No prior frame is held. Both out-pointers are valid for the whole call.
        let acquired = unsafe {
            self.duplication
                .AcquireNextFrame(ACQUIRE_TIMEOUT_MS, &mut info, &mut resource)
        };
        match acquired {
            Ok(()) => {
                // Only successful acquisition constructs a ReleaseFrame owner.
                let frame = AcquiredFrame(Some(&self.duplication));
                let copy_desktop = self.staging.is_none() || info.LastPresentTime != 0;
                let image = Staging::capture(
                    &self.device,
                    &self.context,
                    &self.description,
                    &mut self.staging,
                    resource,
                    copy_desktop,
                    deadline,
                );
                // Explicit release propagates failure, even if decoding also failed.
                // All commands using the acquired surface have been submitted;
                // readback uses our independent staging texture after a timeout.
                frame.release()?;
                image
            }
            Err(error) if error.code() == DXGI_ERROR_WAIT_TIMEOUT && self.staging.is_some() => {
                // A timeout acquires nothing. Never call ReleaseFrame for it.
                match self.staging.as_ref() {
                    Some(staging) => staging.decode(&self.context, &self.description, deadline),
                    None => Err(CaptureError::invalid(
                        "staging cache",
                        "No desktop image is available",
                    )),
                }
            }
            Err(error) => Err(CaptureError::native(
                "IDXGIOutputDuplication::AcquireNextFrame",
                if error.code() == DXGI_ERROR_WAIT_TIMEOUT {
                    "No initial desktop image arrived before the capture timeout"
                } else {
                    "Could not acquire a desktop image"
                },
                error,
            )),
        }
    }
}

fn find_output(
    factory: &IDXGIFactory1,
    monitor: usize,
) -> Result<(IDXGIAdapter1, IDXGIOutput1, OutputDescription), CaptureError> {
    for adapter_index in 0..u32::MAX {
        // SAFETY: factory is live; the index is a value and returned references are owned.
        let adapter = match unsafe { factory.EnumAdapters1(adapter_index) } {
            Ok(adapter) => adapter,
            Err(error) if error.code() == DXGI_ERROR_NOT_FOUND => break,
            Err(error) => {
                return Err(CaptureError::native(
                    "IDXGIFactory1::EnumAdapters1",
                    "Could not enumerate display adapters",
                    error,
                ));
            }
        };
        for output_index in 0..u32::MAX {
            // SAFETY: adapter is live and owns all outputs returned from enumeration.
            let output = match unsafe { adapter.EnumOutputs(output_index) } {
                Ok(output) => output,
                Err(error) if error.code() == DXGI_ERROR_NOT_FOUND => break,
                Err(error) => {
                    return Err(CaptureError::native(
                        "IDXGIAdapter::EnumOutputs",
                        "Could not enumerate adapter displays",
                        error,
                    ));
                }
            };
            // SAFETY: output is live; windows-rs provides the descriptor storage.
            let desc = unsafe { output.GetDesc() }.map_err(|error| {
                CaptureError::native(
                    "IDXGIOutput::GetDesc",
                    "Could not query display geometry",
                    error,
                )
            })?;
            if desc.AttachedToDesktop.as_bool() && desc.Monitor.0 as usize == monitor {
                let output = output.cast::<IDXGIOutput1>().map_err(|error| {
                    CaptureError::native(
                        "QueryInterface(IDXGIOutput1)",
                        "The display does not support desktop duplication",
                        error,
                    )
                })?;
                return Ok((adapter, output, OutputDescription::new(desc)));
            }
        }
    }
    Err(CaptureError::invalid(
        "DXGI output selection",
        "No attached DXGI output matches the game display",
    ))
}

struct Staging {
    texture: ID3D11Texture2D,
    width: u32,
    height: u32,
}

impl Staging {
    fn capture(
        device: &ID3D11Device,
        context: &ID3D11DeviceContext,
        description: &OutputDescription,
        cache: &mut Option<Self>,
        resource: Option<IDXGIResource>,
        copy_desktop: bool,
        deadline: Instant,
    ) -> Result<CapturedFrame, CaptureError> {
        if copy_desktop {
            let resource = resource.ok_or_else(|| {
                CaptureError::invalid(
                    "IDXGIOutputDuplication::AcquireNextFrame",
                    "No desktop resource was returned",
                )
            })?;
            let source = resource.cast::<ID3D11Texture2D>().map_err(|error| {
                CaptureError::native(
                    "QueryInterface(ID3D11Texture2D)",
                    "Could not read the desktop texture",
                    error,
                )
            })?;
            let mut desc = D3D11_TEXTURE2D_DESC::default();
            // SAFETY: source is a live texture and desc is valid writable storage.
            unsafe { source.GetDesc(&mut desc) };
            description.validate_texture(&desc)?;
            if cache.is_none() {
                let staging_desc = D3D11_TEXTURE2D_DESC {
                    Width: desc.Width,
                    Height: desc.Height,
                    MipLevels: 1,
                    ArraySize: 1,
                    Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    SampleDesc: DXGI_SAMPLE_DESC {
                        Count: 1,
                        Quality: 0,
                    },
                    Usage: D3D11_USAGE_STAGING,
                    BindFlags: 0,
                    CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
                    MiscFlags: 0,
                };
                let mut texture = None;
                // SAFETY: the checked BGRA8, single-subresource descriptor requests
                // CPU-read staging only, and texture is valid out-pointer storage.
                unsafe { device.CreateTexture2D(&staging_desc, None, Some(&mut texture)) }
                    .map_err(|error| {
                        CaptureError::native(
                            "ID3D11Device::CreateTexture2D",
                            "Could not allocate desktop staging texture",
                            error,
                        )
                    })?;
                let texture = texture.ok_or_else(|| {
                    CaptureError::invalid(
                        "ID3D11Device::CreateTexture2D",
                        "No desktop staging texture was returned",
                    )
                })?;
                *cache = Some(Self {
                    texture,
                    width: desc.Width,
                    height: desc.Height,
                });
            }
            if let Some(staging) = cache.as_ref() {
                if staging.width != desc.Width || staging.height != desc.Height {
                    return Err(CaptureError::invalid(
                        "DXGI desktop geometry",
                        "Desktop texture dimensions changed during capture",
                    ));
                }
                // SAFETY: separate textures on the same device, identical checked
                // format/dimensions/subresource/sample layout, neither mapped.
                // The source is acquired while submitting this copy; the D3D11
                // command stream retains its resources until execution finishes.
                unsafe { context.CopyResource(&staging.texture, &source) };
            }
        }
        match cache.as_ref() {
            Some(staging) => staging.decode(context, description, deadline),
            None => Err(CaptureError::invalid(
                "staging cache",
                "No desktop image is available",
            )),
        }
    }

    fn decode(
        &self,
        context: &ID3D11DeviceContext,
        description: &OutputDescription,
        deadline: Instant,
    ) -> Result<CapturedFrame, CaptureError> {
        let mapping = MappedTexture::new(context, self, deadline)?;
        let image = decode_bgra(
            mapping.rows()?,
            self.width,
            self.height,
            description.rotation()?,
        )
        .map_err(|message| CaptureError::invalid("BGRA desktop conversion", message))?;
        Ok(CapturedFrame {
            width: image.width(),
            height: image.height(),
            image,
            origin_x: description.bounds.left,
            origin_y: description.bounds.top,
        })
    }
}

struct AcquiredFrame<'a>(Option<&'a IDXGIOutputDuplication>);

impl AcquiredFrame<'_> {
    fn release(mut self) -> Result<(), CaptureError> {
        if let Some(duplication) = self.0.take() {
            // SAFETY: constructed only after successful AcquireNextFrame. Taking
            // the reference prevents Drop from releasing a second time on error.
            unsafe { duplication.ReleaseFrame() }.map_err(|error| {
                CaptureError::native(
                    "IDXGIOutputDuplication::ReleaseFrame",
                    "Could not release the desktop frame",
                    error,
                )
            })?;
        }
        Ok(())
    }
}

impl Drop for AcquiredFrame<'_> {
    fn drop(&mut self) {
        if let Some(duplication) = self.0.take() {
            // SAFETY: this guard is the sole owner of the successful acquisition.
            // Normal paths explicitly release; this path handles unwinding, which
            // also drops the local Session and cannot leave a failed session cached.
            let _ = unsafe { duplication.ReleaseFrame() };
        }
    }
}

struct MappedTexture<'a> {
    context: &'a ID3D11DeviceContext,
    staging: &'a Staging,
    mapped: D3D11_MAPPED_SUBRESOURCE,
}

impl<'a> MappedTexture<'a> {
    fn new(
        context: &'a ID3D11DeviceContext,
        staging: &'a Staging,
        deadline: Instant,
    ) -> Result<Self, CaptureError> {
        let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
        let mut flushed = false;
        loop {
            // SAFETY: staging has one CPU-readable subresource on this context's
            // device, is not mapped, and mapped is valid out-pointer storage.
            // DO_NOT_WAIT reports unfinished GPU work rather than blocking on it.
            let result = unsafe {
                context.Map(
                    &staging.texture,
                    0,
                    D3D11_MAP_READ,
                    D3D11_MAP_FLAG_DO_NOT_WAIT.0 as u32,
                    Some(&mut mapped),
                )
            };
            match result {
                Ok(()) => {
                    // Successful Map immediately creates its sole Unmap owner.
                    return Ok(Self {
                        context,
                        staging,
                        mapped,
                    });
                }
                Err(error) if error.code() == DXGI_ERROR_WAS_STILL_DRAWING => {
                    if !flushed {
                        // SAFETY: exclusive access serializes this live immediate
                        // context. Flush submits pending copies without waiting
                        // for their completion, including on an expired deadline.
                        unsafe { context.Flush() };
                        flushed = true;
                    }
                    let remaining = deadline.saturating_duration_since(Instant::now());
                    if remaining.is_zero() {
                        return Err(CaptureError::native(
                            "ID3D11DeviceContext::Map",
                            "Desktop readback did not finish before the capture timeout",
                            error,
                        ));
                    }
                    std::thread::sleep(remaining.min(Duration::from_millis(1)));
                }
                Err(error) => {
                    return Err(CaptureError::native(
                        "ID3D11DeviceContext::Map",
                        "Could not map the desktop image",
                        error,
                    ));
                }
            }
        }
    }

    fn rows(&self) -> Result<impl Iterator<Item = &[u8]> + '_, CaptureError> {
        let invalid = |message| CaptureError::invalid("mapped desktop layout", message);
        let pitch = usize::try_from(self.mapped.RowPitch)
            .map_err(|_| invalid("Invalid mapped desktop buffer"))?;
        // SAFETY: successful Map of the copied BGRA8 staging texture provides a
        // single allocation with initialized pixel rows at RowPitch intervals.
        // Map excludes GPU writes, and this guard's borrow keeps the mapping live
        // and immutable until every row slice is finished. Only pixel bytes are
        // borrowed; driver padding need not be initialized. DepthPitch is not a
        // Texture2D buffer length and is deliberately unused.
        unsafe {
            bgra_rows(
                self,
                self.mapped.pData.cast(),
                self.staging.width,
                self.staging.height,
                pitch,
            )
        }
        .map_err(invalid)
    }
}

impl Drop for MappedTexture<'_> {
    fn drop(&mut self) {
        // SAFETY: construction requires successful Map of this exact texture and
        // subresource. No explicit unmap or second owner exists; all slices ended.
        unsafe { self.context.Unmap(&self.staging.texture, 0) };
    }
}
