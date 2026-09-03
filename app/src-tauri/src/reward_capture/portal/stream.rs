//! Frame buffers, the conversion into the image the crop path expects, and the PipeWire pump.

use std::cell::{Cell, RefCell};
use std::os::unix::fs::FileExt;
use std::rc::Rc;
use std::time::Duration;

use pipewire as pw;
use pw::loop_::Timeout;
use pw::{properties::properties, spa};
use spa::param::format::{MediaSubtype, MediaType};
use spa::param::format_utils;
use spa::param::video::{VideoFormat, VideoInfoRaw};
use spa::pod::Pod;

/// Which way round the colour channels arrive.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PixelOrder {
    Rgbx,
    Bgrx,
}

/// One captured frame, exactly as PipeWire handed it over.
pub struct FrameBuffer {
    pub width: u32,
    pub height: u32,
    /// Bytes per row, which is padded to the compositor's alignment and is NOT `width * 4`.
    pub stride: usize,
    pub format: PixelOrder,
    pub bytes: Vec<u8>,
}

/// Copy a captured frame into the `RgbaImage` the crop path expects.
///
/// Row by row, because the stride is padded: copying the buffer wholesale skews every row after
/// the first and the card crops land on nothing. A short buffer leaves the remainder at zero
/// rather than panicking -- a torn frame should cost one poll, not the process.
pub fn frame_to_rgba(frame: &FrameBuffer) -> image::RgbaImage {
    let mut image = image::RgbaImage::new(frame.width, frame.height);
    for y in 0..frame.height as usize {
        // Hoisted out of the inner loop, and checked rather than wrapped: a `stride * height`
        // that overflows would otherwise wrap to a small offset and read the wrong rows.
        let Some(row) = y.checked_mul(frame.stride) else {
            break;
        };
        for x in 0..frame.width as usize {
            let Some(offset) = x.checked_mul(4).and_then(|column| row.checked_add(column)) else {
                break;
            };
            // Only the three colour bytes are read, so only those three need to be present.
            // Guarding on `offset + 3` instead would drop a pixel whose alpha byte is the one
            // missing byte, putting a black pixel one step before the real truncation point.
            if offset + 2 >= frame.bytes.len() {
                break;
            }
            let (first, second, third) = (
                frame.bytes[offset],
                frame.bytes[offset + 1],
                frame.bytes[offset + 2],
            );
            let (red, green, blue) = match frame.format {
                PixelOrder::Rgbx => (first, second, third),
                PixelOrder::Bgrx => (third, second, first),
            };
            // Alpha is forced opaque: the `x` byte in RGBx/BGRx is padding, and a compositor
            // that leaves it at zero would otherwise produce a fully transparent frame.
            image.put_pixel(x as u32, y as u32, image::Rgba([red, green, blue, 255]));
        }
    }
    image
}

/// What the format negotiation settled on, carried into the `process` callback.
#[derive(Default)]
struct Negotiated {
    width: u32,
    height: u32,
    format: Option<PixelOrder>,
}

/// A held-open PipeWire stream on one portal node.
///
/// One stream for the poller's lifetime rather than one per poll: the poller runs every two
/// seconds, and renegotiating each time would mean a fresh session -- and on a strict portal, a
/// fresh prompt -- twice a minute.
pub struct NodeStream {
    mainloop: pw::main_loop::MainLoopRc,
    latest: Rc<RefCell<Option<FrameBuffer>>>,
    /// Set when PipeWire reports the stream torn down, so a dead node is distinguishable from a
    /// merely slow one: both otherwise present as "no frame yet" and would keep the session
    /// marked live forever.
    dead: Rc<Cell<bool>>,
    // Both are kept alive for their side effects: dropping the listener unregisters the callbacks
    // and dropping the stream disconnects the node.
    _stream: pw::stream::StreamRc,
    _listener: pw::stream::StreamListener<Negotiated>,
}

impl NodeStream {
    pub fn open(node_id: u32) -> Result<Self, &'static str> {
        pw::init();
        let mainloop = pw::main_loop::MainLoopRc::new(None)
            .map_err(|_| "could not start the pipewire loop")?;
        let context = pw::context::ContextRc::new(&mainloop, None)
            .map_err(|_| "could not create a pipewire context")?;
        let core = context
            .connect_rc(None)
            .map_err(|_| "could not connect to pipewire")?;
        let stream = pw::stream::StreamRc::new(
            core,
            "tennoscope-reward-capture",
            properties! {
                *pw::keys::MEDIA_TYPE => "Video",
                *pw::keys::MEDIA_CATEGORY => "Capture",
                *pw::keys::MEDIA_ROLE => "Screen",
            },
        )
        .map_err(|_| "could not create a pipewire stream")?;

        let latest: Rc<RefCell<Option<FrameBuffer>>> = Rc::new(RefCell::new(None));
        let latest_in_process = Rc::clone(&latest);
        // Terminal-state flag, shared with the `state_changed` callback below.
        let dead = Rc::new(Cell::new(false));
        let dead_in_state = Rc::clone(&dead);

        let listener = stream
            .add_local_listener_with_user_data(Negotiated::default())
            .state_changed(move |_stream, _negotiated, old, new| {
                // `Paused` is normal -- the portal parks the stream when it has nothing to send --
                // so only an error or a real teardown counts. A transition into `Unconnected` is
                // only a teardown if we were connected before: the stream starts `Unconnected`,
                // and treating the startup edge as death would kill every stream on open.
                let terminal = matches!(new, pw::stream::StreamState::Error(_))
                    || (matches!(new, pw::stream::StreamState::Unconnected)
                        && !matches!(old, pw::stream::StreamState::Unconnected));
                if terminal {
                    log::warn!("[DEBUG-capture] portal stream died: {old:?} -> {new:?}");
                    dead_in_state.set(true);
                }
            })
            .param_changed(|_stream, negotiated, id, param| {
                let Some(param) = param else { return };
                if id != spa::param::ParamType::Format.as_raw() {
                    return;
                }
                let Ok((media_type, media_subtype)) = format_utils::parse_format(param) else {
                    return;
                };
                if media_type != MediaType::Video || media_subtype != MediaSubtype::Raw {
                    return;
                }
                let mut info = VideoInfoRaw::default();
                if info.parse(param).is_ok() {
                    let size = info.size();
                    negotiated.width = size.width;
                    negotiated.height = size.height;
                    negotiated.format = match info.format() {
                        VideoFormat::BGRx | VideoFormat::BGRA => Some(PixelOrder::Bgrx),
                        VideoFormat::RGBx | VideoFormat::RGBA => Some(PixelOrder::Rgbx),
                        _ => None,
                    };
                    log::info!(
                        "[DEBUG-capture] portal negotiated {}x{} format={:?}",
                        size.width,
                        size.height,
                        info.format()
                    );
                }
            })
            .process(move |stream, negotiated| {
                let Some(mut buffer) = stream.dequeue_buffer() else {
                    return;
                };
                let datas = buffer.datas_mut();
                if datas.is_empty() {
                    return;
                }
                let Some(format) = negotiated.format else {
                    return;
                };
                let data = &mut datas[0];

                // `Data::chunk()` asserts the chunk pointer is non-null, so on a malformed buffer
                // it panics -- and this is a callback invoked from C, where an unwind is a crash in
                // the middle of someone's game. The raw pointer is reachable safely (only
                // dereferencing it would need `unsafe`), so it gets checked first.
                if data.as_raw().chunk.is_null() {
                    log::debug!("[DEBUG-capture] skipping a pipewire buffer with no chunk");
                    return;
                }

                let kind = data.type_();
                let stride = data.chunk().stride().unsigned_abs() as usize;
                let chunk_offset = data.chunk().offset() as usize;
                let chunk_size = data.chunk().size() as usize;
                let raw = data.as_raw();
                let (maxsize, fd, mapoffset) =
                    (raw.maxsize as usize, raw.fd, raw.mapoffset as usize);
                if maxsize == 0 || stride == 0 {
                    return;
                }
                // Only an fd-backed buffer can be read this way. A DMA-BUF fd is a GPU handle
                // whose bytes are not the pixels, so reading it positionally would yield
                // plausible-looking garbage and send OCR chasing ghosts -- better to deliver no
                // frame, which Task 10's notice can explain, than a wrong one.
                if kind != libspa::buffer::DataType::MemFd {
                    log::debug!("[DEBUG-capture] portal handed back a {kind:?} buffer, not MemFd");
                    return;
                }
                let Some(bytes) = read_buffer(fd, mapoffset, maxsize, chunk_offset, chunk_size)
                else {
                    return;
                };
                // Only the newest frame is kept: the poller wants current pixels, and a queue
                // would just hand it a stale reward screen after any hitch.
                *latest_in_process.borrow_mut() = Some(FrameBuffer {
                    width: negotiated.width,
                    height: negotiated.height,
                    stride,
                    format,
                    bytes,
                });
            })
            .register()
            .map_err(|_| "could not register the pipewire listener")?;

        // Packed 32-bit formats only, so the crop maths has a known byte layout.
        let object = spa::pod::object!(
            spa::utils::SpaTypes::ObjectParamFormat,
            spa::param::ParamType::EnumFormat,
            spa::pod::property!(
                spa::param::format::FormatProperties::MediaType,
                Id,
                MediaType::Video
            ),
            spa::pod::property!(
                spa::param::format::FormatProperties::MediaSubtype,
                Id,
                MediaSubtype::Raw
            ),
            spa::pod::property!(
                spa::param::format::FormatProperties::VideoFormat,
                Choice,
                Enum,
                Id,
                VideoFormat::BGRx,
                VideoFormat::BGRx,
                VideoFormat::RGBx,
                VideoFormat::BGRA,
                VideoFormat::RGBA,
            ),
        );
        let values: Vec<u8> = spa::pod::serialize::PodSerializer::serialize(
            std::io::Cursor::new(Vec::new()),
            &spa::pod::Value::Object(object),
        )
        .map_err(|_| "could not describe the wanted video format")?
        .0
        .into_inner();
        let mut params = [Pod::from_bytes(&values).ok_or("malformed format description")?];

        stream
            .connect(
                spa::utils::Direction::Input,
                Some(node_id),
                pw::stream::StreamFlags::AUTOCONNECT | pw::stream::StreamFlags::MAP_BUFFERS,
                &mut params,
            )
            .map_err(|_| "could not connect to the portal's stream")?;

        Ok(Self {
            mainloop,
            latest,
            dead,
            _stream: stream,
            _listener: listener,
        })
    }

    /// Pump the loop until a frame is available, the stream dies, or `deadline` passes.
    ///
    /// The cold run measured a first frame arriving well after the handshake returned, so this
    /// waits rather than assuming the buffer is ready.
    ///
    /// A dead stream reports [`super::ERR_SESSION_ENDED`] rather than a timeout, because the two
    /// need opposite handling: a timeout is a frame worth retrying on the same session, while a
    /// teardown means the session is gone and the user has to be offered authorization again.
    pub fn next_frame(&self, deadline: Duration) -> Result<FrameBuffer, &'static str> {
        let started = std::time::Instant::now();
        loop {
            // Bound in its own statement so the `RefCell` borrow is released before `iterate`
            // runs the `process` callback, which borrows the same cell mutably.
            let frame = self.latest.borrow_mut().take();
            if let Some(frame) = frame {
                return Ok(frame);
            }
            // Checked after the buffer, so a frame that already landed is still delivered even if
            // the stream tore down immediately afterwards.
            if self.dead.get() {
                return Err(super::ERR_SESSION_ENDED);
            }
            if started.elapsed() >= deadline {
                return Err(super::ERR_NO_FRAME);
            }
            // `Timeout` is `pipewire::loop_::Timeout`, an enum with no `From<Duration>`. Slices of
            // 50ms rather than one long block so the deadline stays roughly honest.
            self.mainloop
                .loop_()
                .iterate(Timeout::Finite(Duration::from_millis(50)));
        }
    }
}

/// The largest frame this program will allocate for, matching the KWin backend's budget.
///
/// `maxsize` arrives off the wire too, so bounding a chunk against it alone is a relative check: a
/// self-consistent buffer declaring gigabytes for both passes it and commits the allocation before
/// a byte is read. Anything past this ceiling is not a desktop frame.
const MAX_FRAME_BYTES: usize = 512 * 1024 * 1024;

/// Read a PipeWire buffer without mapping it.
///
/// `mmap` is the obvious way and is not available here: it needs `unsafe`, and this crate is
/// `#![forbid(unsafe_code)]`, which an inner `#[allow]` cannot relax (E0453). Reopening the
/// buffer's fd through `/proc/self/fd` and reading it positionally gets the same bytes through
/// safe std APIs -- measured against a live portal cast, `8294400/8294400` bytes of real desktop
/// content, the same node and geometry the mmap spike produced.
///
/// Positioned (`read_at`, i.e. `pread`) rather than sequential, because the fd belongs to
/// PipeWire: a plain `read` would move the offset it is using. And a short buffer is just a short
/// read here, where `mmap` past end-of-file raises SIGBUS -- a signal no bounds check prevents and
/// nothing can catch safely, in a process running during someone's game.
fn read_buffer(
    fd: i64,
    mapoffset: usize,
    maxsize: usize,
    chunk_offset: usize,
    chunk_size: usize,
) -> Option<Vec<u8>> {
    // Every bound is checked before the allocation, because `chunk_size` arrives off the wire: a
    // corrupted size field would otherwise be a `vec![0; huge]` before anything looked at it.
    // `maxsize` is the mapping the buffer itself declares, so a chunk that does not fit inside it
    // at its own offset is malformed rather than merely short.
    if chunk_size == 0
        || chunk_size > MAX_FRAME_BYTES
        || chunk_offset.checked_add(chunk_size)? > maxsize
    {
        log::debug!(
            "[DEBUG-capture] rejecting a malformed pipewire chunk: \
             offset={chunk_offset} size={chunk_size} maxsize={maxsize}"
        );
        return None;
    }
    let start = mapoffset.checked_add(chunk_offset)?;
    let file = std::fs::File::open(format!("/proc/self/fd/{fd}")).ok()?;
    read_chunk(&file, start, chunk_size)
}

/// Something readable at an absolute offset.
///
/// `std::fs::File` is the only implementation on the live path. The trait exists because a real fd
/// cannot be made to return `EINTR` or a partial read on demand, and those are precisely the two
/// paths the loop below exists to handle -- so without it they would ship untested.
trait PositionedRead {
    fn read_at(&self, buffer: &mut [u8], offset: u64) -> std::io::Result<usize>;
}

impl PositionedRead for std::fs::File {
    fn read_at(&self, buffer: &mut [u8], offset: u64) -> std::io::Result<usize> {
        FileExt::read_at(self, buffer, offset)
    }
}

/// How many interruptions one chunk read may absorb before the frame is abandoned.
///
/// Counted for the whole read rather than reset on progress. Resetting sounds kinder -- it would
/// never abandon a read that is getting somewhere -- but it re-opens the hole this closes: a reader
/// alternating one byte with a burst of signals would iterate `chunk_size * MAX` times, which for
/// an 8 MB frame is over a hundred million passes and is a hang in all but name. Counting the whole
/// read bounds the loop at `chunk_size + MAX` iterations no matter what the fd does.
const MAX_INTERRUPTIONS: u32 = 16;

/// Fill a `chunk_size` buffer from `reader`, starting at `start`.
///
/// Split from `read_buffer` so the retry and short-read paths are reachable from a test.
fn read_chunk<R: PositionedRead>(reader: &R, start: usize, chunk_size: usize) -> Option<Vec<u8>> {
    let mut bytes = vec![0_u8; chunk_size];
    let mut filled = 0_usize;
    let mut interruptions = 0_u32;
    while filled < chunk_size {
        let offset = u64::try_from(start.checked_add(filled)?).ok()?;
        match reader.read_at(&mut bytes[filled..], offset) {
            // A short buffer keeps what arrived and leaves the rest zeroed: a torn frame should
            // cost one poll, not the process.
            Ok(0) => break,
            Ok(read) => filled += read,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {
                // A signal can legitimately interrupt a read, but only finitely often. Retrying
                // without a bound would spin inside the PipeWire callback, wedging the loop thread
                // and every later poll -- worse than the dropped frame this is trying to avoid.
                interruptions += 1;
                if interruptions > MAX_INTERRUPTIONS {
                    log::debug!(
                        "[DEBUG-capture] giving up on a pipewire chunk after \
                         {MAX_INTERRUPTIONS} interruptions"
                    );
                    return None;
                }
                continue;
            }
            Err(_) => return None,
        }
    }
    Some(bytes)
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::io::Write;
    use std::os::fd::AsRawFd;

    use super::{FrameBuffer, PixelOrder, PositionedRead, frame_to_rgba, read_buffer, read_chunk};

    /// A real fd holding `content`, which is what `read_buffer` is given on the live path.
    fn fd_holding(content: &[u8]) -> std::fs::File {
        let mut file = tempfile::tempfile().expect("a temp file backs the fd");
        file.write_all(content).expect("the pattern is written");
        file.flush().expect("flushed before it is reopened");
        file
    }

    /// The two offsets are additive: `mapoffset` locates the buffer's data area within the fd and
    /// `chunk_offset` locates the valid bytes within that area. Reading from either one alone
    /// returns the wrong pixels, which would decode as a frame shifted by a partial row.
    #[test]
    fn both_offsets_are_honoured_when_reading() {
        // 0,1,2,..63 so any misplaced read is obvious from the values themselves.
        let content: Vec<u8> = (0..64_u8).collect();
        let file = fd_holding(&content);
        let bytes = read_buffer(i64::from(file.as_raw_fd()), 8, 64, 4, 8)
            .expect("a fully readable chunk comes back");
        // mapoffset 8 + chunk_offset 4 = file offset 12.
        assert_eq!(bytes, vec![12, 13, 14, 15, 16, 17, 18, 19]);
    }

    /// A torn frame must cost one poll, not the process: a chunk the fd cannot fully satisfy
    /// keeps the bytes that did arrive and leaves the rest zeroed.
    #[test]
    fn a_short_read_zero_fills_the_remainder() {
        let file = fd_holding(&[7_u8; 16]);
        let bytes = read_buffer(i64::from(file.as_raw_fd()), 0, 32, 0, 32)
            .expect("a short read still yields a buffer");
        assert_eq!(bytes.len(), 32);
        assert_eq!(&bytes[..16], &[7_u8; 16]);
        // The tail is zero rather than uninitialised or truncated.
        assert_eq!(&bytes[16..], &[0_u8; 16]);
    }

    /// `chunk_size` arrives straight off the wire. A malformed buffer claiming more than the
    /// mapping holds must be rejected BEFORE the allocation -- otherwise a bogus gigabyte-scale
    /// size is a gigabyte-scale `vec![0; n]` in a process running during someone's game.
    #[test]
    fn a_chunk_larger_than_the_mapping_is_rejected() {
        let file = fd_holding(&[1_u8; 16]);
        let fd = i64::from(file.as_raw_fd());
        // Bigger than maxsize outright.
        assert!(read_buffer(fd, 0, 16, 0, 17).is_none());
        // Fits on its own but not at its offset, which is the subtler malformed case.
        assert!(read_buffer(fd, 0, 16, 12, 8).is_none());
        // Absurd, the shape a corrupted size field actually takes.
        assert!(read_buffer(fd, 0, 16, 0, usize::MAX).is_none());
        // Self-consistent but absurd: the chunk genuinely fits inside the `maxsize` it declares,
        // so every relative bound holds and only the absolute ceiling rejects it. This is the case
        // a purely relative check lets through.
        assert!(read_buffer(fd, 0, 8 * 1024 * 1024 * 1024, 0, 8 * 1024 * 1024 * 1024).is_none());
    }

    /// A reader with scripted behaviour, because a real fd cannot be made to return `EINTR` or a
    /// partial read on demand -- and those are exactly the two paths the retry loop exists for.
    ///
    /// It honours `offset`, which is what makes the contiguity assertion real: a loop that failed
    /// to advance the offset would re-read the same leading bytes every pass.
    struct ScriptedReader {
        content: Vec<u8>,
        /// The most any single call will hand back, so a full read needs several passes.
        max_per_call: usize,
        /// Leading calls that fail with `EINTR` before any data is returned.
        interruptions_left: Cell<u32>,
        /// Every call fails with `EINTR`, the persistent-signal case.
        always_interrupt: bool,
        calls: Cell<u32>,
    }

    impl ScriptedReader {
        fn partial(content: Vec<u8>, max_per_call: usize) -> Self {
            Self {
                content,
                max_per_call,
                interruptions_left: Cell::new(0),
                always_interrupt: false,
                calls: Cell::new(0),
            }
        }
    }

    impl PositionedRead for ScriptedReader {
        fn read_at(&self, buffer: &mut [u8], offset: u64) -> std::io::Result<usize> {
            self.calls.set(self.calls.get() + 1);
            // A missing retry bound would spin here forever. Panicking turns that into a visible
            // test failure instead of a hung suite.
            assert!(
                self.calls.get() <= 1_000,
                "read_chunk retried over 1000 times; the interruption bound is missing"
            );
            if self.always_interrupt || self.interruptions_left.get() > 0 {
                if !self.always_interrupt {
                    self.interruptions_left
                        .set(self.interruptions_left.get() - 1);
                }
                return Err(std::io::Error::from(std::io::ErrorKind::Interrupted));
            }
            let start = usize::try_from(offset).expect("a test offset fits in usize");
            if start >= self.content.len() {
                return Ok(0);
            }
            let take = (self.content.len() - start)
                .min(buffer.len())
                .min(self.max_per_call);
            buffer[..take].copy_from_slice(&self.content[start..start + take]);
            Ok(take)
        }
    }

    /// A chunk that arrives in several passes lands contiguously. Destination slice and file
    /// offset must advance by the same amount: advancing one without the other duplicates or skips
    /// a run of bytes, which decodes as a band of repeated or missing pixels rather than as an
    /// obvious failure.
    #[test]
    fn a_chunk_read_in_several_passes_is_still_contiguous() {
        let content: Vec<u8> = (0..64_u8).collect();
        let reader = ScriptedReader::partial(content.clone(), 8);
        let bytes = read_chunk(&reader, 0, 64).expect("the whole content is readable");
        assert_eq!(bytes, content);
        // 64 bytes at 8 per call: the loop genuinely iterated rather than reading it in one go.
        assert_eq!(
            reader.calls.get(),
            8,
            "the read did not take several passes"
        );
    }

    /// A signal may legitimately interrupt a read. Those retries must still succeed, or a frame
    /// would be dropped for a condition that resolves on its own.
    #[test]
    fn a_transient_interruption_is_retried() {
        let content: Vec<u8> = (0..16_u8).collect();
        let mut reader = ScriptedReader::partial(content.clone(), 16);
        reader.interruptions_left = Cell::new(3);
        let bytes = read_chunk(&reader, 0, 16).expect("a transient EINTR does not lose the frame");
        assert_eq!(bytes, content);
    }

    /// A signal arriving faster than the read completes must not spin. Retrying without a bound
    /// would hang inside the PipeWire callback, wedging the loop thread and every later poll --
    /// worse than the dropped frame the retry was protecting.
    #[test]
    fn a_persistent_interruption_gives_up_rather_than_spinning() {
        let reader = ScriptedReader {
            content: vec![1_u8; 16],
            max_per_call: 16,
            interruptions_left: Cell::new(0),
            always_interrupt: true,
            calls: Cell::new(0),
        };
        assert!(
            read_chunk(&reader, 0, 16).is_none(),
            "a persistent EINTR must give up"
        );
    }

    /// PipeWire strides are padded to the compositor's alignment, so `stride != width * 4` in
    /// general. Ignoring the padding skews every row and the crops land on nothing.
    #[test]
    fn a_padded_stride_does_not_skew_the_image() {
        // 2x2 image whose rows are padded to 12 bytes instead of 8.
        let mut bytes = vec![0_u8; 24];
        // Row 0: red pixel, then green.
        bytes[0..4].copy_from_slice(&[255, 0, 0, 255]);
        bytes[4..8].copy_from_slice(&[0, 255, 0, 255]);
        // Row 1 begins at the stride, not at width * 4.
        bytes[12..16].copy_from_slice(&[0, 0, 255, 255]);
        bytes[16..20].copy_from_slice(&[255, 255, 255, 255]);
        let image = frame_to_rgba(&FrameBuffer {
            width: 2,
            height: 2,
            stride: 12,
            format: PixelOrder::Rgbx,
            bytes,
        });
        assert_eq!(image.get_pixel(0, 0).0, [255, 0, 0, 255]);
        assert_eq!(image.get_pixel(1, 0).0, [0, 255, 0, 255]);
        assert_eq!(image.get_pixel(0, 1).0, [0, 0, 255, 255]);
        assert_eq!(image.get_pixel(1, 1).0, [255, 255, 255, 255]);
    }

    /// BGRx is the other format compositors hand back. Getting the order wrong tints the whole
    /// frame and every OCR read degrades without anything looking obviously broken.
    #[test]
    fn bgrx_channels_are_put_back_in_order() {
        let image = frame_to_rgba(&FrameBuffer {
            width: 1,
            height: 1,
            stride: 4,
            format: PixelOrder::Bgrx,
            // B, G, R, x -- a pure red pixel in BGRx.
            bytes: vec![0, 0, 255, 255],
        });
        assert_eq!(image.get_pixel(0, 0).0, [255, 0, 0, 255]);
    }

    /// A short buffer must not panic: a torn frame is a dropped poll, not a crash.
    #[test]
    fn a_truncated_buffer_yields_a_full_sized_image() {
        let image = frame_to_rgba(&FrameBuffer {
            width: 4,
            height: 4,
            stride: 16,
            format: PixelOrder::Rgbx,
            bytes: vec![255; 20],
        });
        assert_eq!(image.dimensions(), (4, 4));
    }

    /// A pixel whose three colour bytes are the last three bytes of the buffer is readable, and
    /// must actually be read. Pinned because the obvious guard -- `offset + 3 >= len` -- is off by
    /// one and silently drops it, which is a black pixel appearing at the truncation point rather
    /// than at the first genuinely missing byte.
    #[test]
    fn a_pixel_ending_exactly_at_the_buffer_end_is_still_read() {
        let image = frame_to_rgba(&FrameBuffer {
            width: 1,
            height: 2,
            stride: 4,
            format: PixelOrder::Rgbx,
            // Row 1's alpha byte is missing, but its R, G and B are all present.
            bytes: vec![0, 0, 0, 255, 10, 20, 30],
        });
        assert_eq!(image.get_pixel(0, 1).0, [10, 20, 30, 255]);
    }
}
