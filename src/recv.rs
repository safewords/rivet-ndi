//! Receiving (`NDIlib_recv_*`).

use std::sync::Arc;
use std::time::Duration;

use crate::convert::{self, ColorInfo, FourCC, Picture};
use crate::ffi::{self, Lib};
use crate::find::{RawSource, Source, millis};
use crate::{Error, Result, c_string, cstr_to_string, defined_timestamp};

/// The pixel layouts the receiver asks the runtime for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ColorFormat {
    /// Whatever the runtime decodes to most cheaply: UYVY (UYVA with
    /// alpha), 8-bit. The default.
    #[default]
    Fastest,
    /// The best the source has: P216 (PA16 with alpha) for a source sent
    /// at more than 8 bits, else as `Fastest`.
    Best,
    /// UYVY without alpha, BGRA with it.
    UyvyBgra,
}

/// The stream the receiver asks the sender for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Bandwidth {
    /// The full-quality stream. The default.
    #[default]
    Highest,
    /// The sender's low-bandwidth proxy, where it offers one.
    Lowest,
    /// Audio (and metadata) only.
    AudioOnly,
    /// Metadata only.
    MetadataOnly,
}

/// How a receiver connects.
#[derive(Debug, Clone, Default)]
pub struct ReceiverOptions {
    pub color_format: ColorFormat,
    pub bandwidth: Bandwidth,
    /// Take interlaced sources as fields (`false`, the default, has the
    /// runtime weave them into progressive frames).
    pub allow_fields: bool,
    /// The name this receiver shows the sender and NDI tools.
    pub name: Option<String>,
}

pub(crate) struct RecvInner {
    lib: Arc<Lib>,
    instance: ffi::Instance,
}

// SAFETY: the SDK allows capture on one thread and frees on any.
unsafe impl Send for RecvInner {}
unsafe impl Sync for RecvInner {}

impl Drop for RecvInner {
    fn drop(&mut self) {
        // SAFETY: created by `recv_create_v3`, destroyed once, after every
        // frame (each holds an `Arc` to this) has been freed.
        unsafe { (self.lib.recv_destroy)(self.instance) }
    }
}

/// A connection to one source.
pub struct Receiver {
    inner: Arc<RecvInner>,
    source: Source,
}

/// What one [`Receiver::capture`] brought.
pub enum Capture {
    /// Nothing arrived within the timeout.
    None,
    Video(VideoFrame),
    Audio(AudioFrame),
    /// A metadata frame (XML).
    Metadata(String),
    /// The connection's settings changed (tally, PTZ capability, ...).
    StatusChange,
    /// The sender behind the source name changed (NDI 6).
    SourceChange,
}

impl Receiver {
    pub(crate) fn new(lib: Arc<Lib>, source: &Source, options: &ReceiverOptions) -> Result<Self> {
        let raw = RawSource::new(source)?;
        let name = options
            .name
            .as_deref()
            .map(|n| c_string(n, "the NDI receiver name"))
            .transpose()?;
        let create = ffi::RecvCreateV3 {
            source_to_connect_to: raw.raw,
            color_format: match options.color_format {
                ColorFormat::Fastest => ffi::RECV_COLOR_FORMAT_FASTEST,
                ColorFormat::Best => ffi::RECV_COLOR_FORMAT_BEST,
                ColorFormat::UyvyBgra => ffi::RECV_COLOR_FORMAT_UYVY_BGRA,
            },
            bandwidth: match options.bandwidth {
                Bandwidth::Highest => ffi::RECV_BANDWIDTH_HIGHEST,
                Bandwidth::Lowest => ffi::RECV_BANDWIDTH_LOWEST,
                Bandwidth::AudioOnly => ffi::RECV_BANDWIDTH_AUDIO_ONLY,
                Bandwidth::MetadataOnly => ffi::RECV_BANDWIDTH_METADATA_ONLY,
            },
            allow_video_fields: options.allow_fields,
            p_ndi_recv_name: name.as_ref().map_or(std::ptr::null(), |n| n.as_ptr()),
        };
        // SAFETY: every string outlives the call, which copies them.
        let instance = unsafe { (lib.recv_create_v3)(&create) };
        if instance.is_null() {
            return Err(Error::CreateFailed("NDIlib_recv_create_v3"));
        }
        Ok(Self {
            inner: Arc::new(RecvInner { lib, instance }),
            source: source.clone(),
        })
    }

    /// The source this receiver connects to.
    pub fn source(&self) -> &Source {
        &self.source
    }

    /// How many connections the receiver has (0 until the sender answers).
    pub fn connections(&self) -> usize {
        // SAFETY: a live instance.
        unsafe { (self.inner.lib.recv_get_no_connections)(self.inner.instance) }.max(0) as usize
    }

    /// Wait up to `timeout` for the next frame of any kind.
    pub fn capture(&mut self, timeout: Duration) -> Result<Capture> {
        let lib = &self.inner.lib;
        let mut video = ffi::VideoFrameV2::zeroed();
        let mut audio = ffi::AudioFrameV2::zeroed();
        let mut meta = ffi::MetadataFrame::zeroed();
        // SAFETY: a live instance and three frames for the runtime to fill;
        // whichever it fills is handed to an owner that frees it.
        let kind = unsafe {
            (lib.recv_capture_v2)(
                self.inner.instance,
                &mut video,
                &mut audio,
                &mut meta,
                millis(timeout),
            )
        };
        Ok(match kind {
            ffi::FRAME_TYPE_VIDEO => Capture::Video(VideoFrame {
                inner: Arc::clone(&self.inner),
                raw: video,
            }),
            ffi::FRAME_TYPE_AUDIO => Capture::Audio(AudioFrame {
                inner: Arc::clone(&self.inner),
                raw: audio,
            }),
            ffi::FRAME_TYPE_METADATA => {
                // SAFETY: a NUL-terminated string the runtime owns until freed.
                let text = unsafe { cstr_to_string(meta.p_data) }.unwrap_or_default();
                unsafe { (lib.recv_free_metadata)(self.inner.instance, &meta) };
                Capture::Metadata(text)
            }
            ffi::FRAME_TYPE_STATUS_CHANGE => Capture::StatusChange,
            ffi::FRAME_TYPE_SOURCE_CHANGE => Capture::SourceChange,
            ffi::FRAME_TYPE_ERROR => return Err(Error::ConnectionLost),
            _ => Capture::None,
        })
    }
}

/// A received video frame. The pixels are the runtime's, returned to it on
/// drop; [`Self::to_picture`] copies them out.
pub struct VideoFrame {
    inner: Arc<RecvInner>,
    raw: ffi::VideoFrameV2,
}

// SAFETY: the frame's buffer is the runtime's until freed, from any thread.
unsafe impl Send for VideoFrame {}

impl VideoFrame {
    pub fn width(&self) -> u32 {
        self.raw.xres.max(0) as u32
    }

    pub fn height(&self) -> u32 {
        self.raw.yres.max(0) as u32
    }

    pub fn fourcc(&self) -> FourCC {
        FourCC::from_raw(self.raw.fourcc)
    }

    /// Bytes from one row of the first plane to the next.
    pub fn stride(&self) -> usize {
        self.raw.line_stride_in_bytes.max(0) as usize
    }

    /// The frame rate as the sender declared it, `(numerator, denominator)`
    /// — `(30000, 1001)` for 29.97.
    pub fn frame_rate(&self) -> (u32, u32) {
        (
            self.raw.frame_rate_n.max(0) as u32,
            self.raw.frame_rate_d.max(0) as u32,
        )
    }

    /// The picture's display aspect ratio, `None` when the sender left it
    /// to the pixel dimensions (square pixels).
    pub fn picture_aspect_ratio(&self) -> Option<f32> {
        let r = self.raw.picture_aspect_ratio;
        (r.is_finite() && r > 0.0).then_some(r)
    }

    /// Whether this is a progressive frame (rather than an interleaved
    /// frame or a single field, which only a receiver made with
    /// `allow_fields` sees).
    pub fn is_progressive(&self) -> bool {
        self.raw.frame_format_type == ffi::FRAME_FORMAT_PROGRESSIVE
    }

    /// The sender's clock when it sent the frame, in 100 ns units since
    /// the Unix epoch; `None` for a sender that stamps none.
    pub fn timestamp(&self) -> Option<i64> {
        defined_timestamp(self.raw.timestamp)
    }

    /// The frame's timecode, in 100 ns units (synthesised by the sender's
    /// runtime unless the sender set its own).
    pub fn timecode(&self) -> i64 {
        self.raw.timecode
    }

    /// The XML metadata attached to the frame.
    pub fn metadata(&self) -> Option<String> {
        // SAFETY: NUL-terminated, the runtime's until the frame is freed.
        unsafe { cstr_to_string(self.raw.p_metadata) }
    }

    /// The colour the frame's metadata declares, when it declares one.
    pub fn color_info(&self) -> Option<ColorInfo> {
        ColorInfo::parse(&self.metadata()?)
    }

    /// The frame's bytes, as the runtime holds them (`fourcc`, rows
    /// `stride` apart).
    pub fn data(&self) -> Result<&[u8]> {
        let len = self
            .fourcc()
            .buffer_len(self.width() as usize, self.height() as usize, self.stride())
            .ok_or(Error::UnsupportedFourCC(self.fourcc()))?;
        if self.raw.p_data.is_null() {
            return Err(Error::BadFrame("a video frame without data".into()));
        }
        // SAFETY: the runtime's buffer for this fourcc, size and stride.
        Ok(unsafe { std::slice::from_raw_parts(self.raw.p_data, len) })
    }

    /// The pixels as a packed planar [`Picture`] ([`convert::to_picture`]).
    pub fn to_picture(&self) -> Result<Picture> {
        convert::to_picture(
            self.fourcc(),
            self.width(),
            self.height(),
            self.stride(),
            self.data()?,
        )
    }
}

impl Drop for VideoFrame {
    fn drop(&mut self) {
        // SAFETY: a frame `recv_capture_v2` filled, freed once.
        unsafe { (self.inner.lib.recv_free_video_v2)(self.inner.instance, &self.raw) }
    }
}

/// A received audio frame: 32-bit float, one plane per channel. Returned
/// to the runtime on drop.
pub struct AudioFrame {
    inner: Arc<RecvInner>,
    raw: ffi::AudioFrameV2,
}

// SAFETY: as for `VideoFrame`.
unsafe impl Send for AudioFrame {}

impl AudioFrame {
    pub fn sample_rate(&self) -> u32 {
        self.raw.sample_rate.max(0) as u32
    }

    pub fn channels(&self) -> usize {
        self.raw.no_channels.max(0) as usize
    }

    /// Samples per channel.
    pub fn samples(&self) -> usize {
        self.raw.no_samples.max(0) as usize
    }

    /// As [`VideoFrame::timestamp`].
    pub fn timestamp(&self) -> Option<i64> {
        defined_timestamp(self.raw.timestamp)
    }

    pub fn timecode(&self) -> i64 {
        self.raw.timecode
    }

    /// Channel `index`'s samples.
    pub fn channel(&self, index: usize) -> &[f32] {
        let stride = self.raw.channel_stride_in_bytes.max(0) as usize;
        if index >= self.channels() || self.raw.p_data.is_null() || !stride.is_multiple_of(4) {
            return &[];
        }
        let n = self.samples().min(stride / 4);
        // SAFETY: the runtime's planes, `channel_stride_in_bytes` apart.
        unsafe {
            std::slice::from_raw_parts(
                (self.raw.p_data as *const u8).add(index * stride) as *const f32,
                n,
            )
        }
    }

    /// Every channel interleaved (`L R L R …`).
    pub fn interleaved(&self) -> Vec<f32> {
        let ch = self.channels();
        let planes: Vec<&[f32]> = (0..ch).map(|c| self.channel(c)).collect();
        let n = planes.iter().map(|p| p.len()).min().unwrap_or(0);
        let mut out = Vec::with_capacity(n * ch);
        for i in 0..n {
            for p in &planes {
                out.push(p[i]);
            }
        }
        out
    }
}

impl Drop for AudioFrame {
    fn drop(&mut self) {
        // SAFETY: a frame `recv_capture_v2` filled, freed once.
        unsafe { (self.inner.lib.recv_free_audio_v2)(self.inner.instance, &self.raw) }
    }
}
