//! Sending (`NDIlib_send_*`).

use std::ffi::CString;
use std::sync::Arc;
use std::time::Duration;

use crate::convert::{FourCC, Picture};
use crate::ffi::{self, Lib};
use crate::find::millis;
use crate::{Error, Result, c_string};

/// How a sender announces itself.
#[derive(Debug, Clone)]
pub struct SenderOptions {
    /// The stream name; receivers see `MACHINE (name)`.
    pub name: String,
    /// The groups to announce in, comma-separated; `None` is the runtime's
    /// default.
    pub groups: Option<String>,
    /// Have the runtime pace video: each `send_video` blocks until the
    /// frame is due at the declared frame rate (default `true`). A sender
    /// reading a file wants this; one forwarding a live feed does not.
    pub clock_video: bool,
    /// The same for audio (default `false`: with both clocked the two
    /// would fight over the pace).
    pub clock_audio: bool,
}

impl SenderOptions {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            groups: None,
            clock_video: true,
            clock_audio: false,
        }
    }
}

/// One video frame to send. The runtime reads `data` during the call.
#[derive(Debug, Clone, Copy)]
pub struct OutgoingVideo<'a> {
    pub fourcc: FourCC,
    pub width: u32,
    pub height: u32,
    /// Bytes from one row of the first plane to the next.
    pub stride: usize,
    pub data: &'a [u8],
    /// `(numerator, denominator)`: `(30000, 1001)` for 29.97.
    pub frame_rate: (u32, u32),
    /// Display aspect ratio; `None` for square pixels.
    pub picture_aspect_ratio: Option<f32>,
    /// 100 ns units; `None` has the runtime synthesise one.
    pub timecode: Option<i64>,
}

/// An announced source.
pub struct Sender {
    lib: Arc<Lib>,
    instance: ffi::Instance,
    name: String,
    /// Scratch for planar audio, reused between frames.
    planar: Vec<f32>,
}

// SAFETY: a sender instance may be used from any one thread at a time.
unsafe impl Send for Sender {}

impl Sender {
    pub(crate) fn new(lib: Arc<Lib>, options: &SenderOptions) -> Result<Self> {
        let name = c_string(&options.name, "the NDI stream name")?;
        let groups = options
            .groups
            .as_deref()
            .map(|g| c_string(g, "the NDI groups"))
            .transpose()?;
        let create = ffi::SendCreate {
            p_ndi_name: name.as_ptr(),
            p_groups: groups.as_ref().map_or(std::ptr::null(), |g| g.as_ptr()),
            clock_video: options.clock_video,
            clock_audio: options.clock_audio,
        };
        // SAFETY: the strings outlive the call, which copies them.
        let instance = unsafe { (lib.send_create)(&create) };
        if instance.is_null() {
            return Err(Error::CreateFailed("NDIlib_send_create"));
        }
        Ok(Self {
            lib,
            instance,
            name: options.name.clone(),
            planar: Vec::new(),
        })
    }

    /// The stream name this sender announced.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Receivers connected now, after waiting up to `timeout` for the first.
    pub fn connections(&self, timeout: Duration) -> usize {
        // SAFETY: a live instance.
        unsafe { (self.lib.send_get_no_connections)(self.instance, millis(timeout)) }.max(0)
            as usize
    }

    /// Send one frame; blocks for pacing when the sender clocks video.
    pub fn send_video(&mut self, frame: &OutgoingVideo<'_>) -> Result<()> {
        let (w, h) = (frame.width as usize, frame.height as usize);
        let need = frame
            .fourcc
            .buffer_len(w, h, frame.stride)
            .ok_or(Error::UnsupportedFourCC(frame.fourcc))?;
        if w == 0 || h == 0 || frame.data.len() < need {
            return Err(Error::BadFrame(format!(
                "{} {w}x{h} stride {}: {} bytes, {need} needed",
                frame.fourcc,
                frame.stride,
                frame.data.len()
            )));
        }
        let (n, d) = frame.frame_rate;
        if n == 0 || d == 0 {
            return Err(Error::BadFrame(format!("frame rate {n}/{d}")));
        }
        let raw = ffi::VideoFrameV2 {
            xres: w as i32,
            yres: h as i32,
            fourcc: frame.fourcc.raw(),
            frame_rate_n: n as i32,
            frame_rate_d: d as i32,
            picture_aspect_ratio: frame.picture_aspect_ratio.unwrap_or(0.0),
            frame_format_type: ffi::FRAME_FORMAT_PROGRESSIVE,
            timecode: frame.timecode.unwrap_or(ffi::SEND_TIMECODE_SYNTHESIZE),
            // The runtime only reads through it during this synchronous call.
            p_data: frame.data.as_ptr() as *mut u8,
            line_stride_in_bytes: frame.stride as i32,
            p_metadata: std::ptr::null(),
            timestamp: 0,
        };
        // SAFETY: `data` holds the whole frame for the duration of the call.
        unsafe { (self.lib.send_send_video_v2)(self.instance, &raw) };
        Ok(())
    }

    /// Send a [`Picture`] at `frame_rate` ([`Picture::to_ndi`] picks the
    /// fourcc).
    pub fn send_picture(
        &mut self,
        picture: &Picture,
        frame_rate: (u32, u32),
        picture_aspect_ratio: Option<f32>,
    ) -> Result<()> {
        let (fourcc, stride, data) = picture.to_ndi()?;
        self.send_video(&OutgoingVideo {
            fourcc,
            width: picture.width,
            height: picture.height,
            stride,
            data: &data,
            frame_rate,
            picture_aspect_ratio,
            timecode: None,
        })
    }

    /// Send interleaved float audio (`L R L R …`, nominal level ±1.0).
    pub fn send_audio(
        &mut self,
        sample_rate: u32,
        channels: usize,
        interleaved: &[f32],
        timecode: Option<i64>,
    ) -> Result<()> {
        if channels == 0 || sample_rate == 0 {
            return Err(Error::BadFrame(format!(
                "audio of {channels} channels at {sample_rate} Hz"
            )));
        }
        let n = interleaved.len() / channels;
        if n == 0 {
            return Ok(());
        }
        self.planar.clear();
        self.planar.resize(n * channels, 0.0);
        for (i, frame) in interleaved.chunks_exact(channels).enumerate() {
            for (c, s) in frame.iter().enumerate() {
                self.planar[c * n + i] = *s;
            }
        }
        let raw = ffi::AudioFrameV2 {
            sample_rate: sample_rate as i32,
            no_channels: channels as i32,
            no_samples: n as i32,
            timecode: timecode.unwrap_or(ffi::SEND_TIMECODE_SYNTHESIZE),
            p_data: self.planar.as_mut_ptr(),
            channel_stride_in_bytes: (n * 4) as i32,
            p_metadata: std::ptr::null(),
            timestamp: 0,
        };
        // SAFETY: the planes are held for the duration of the call.
        unsafe { (self.lib.send_send_audio_v2)(self.instance, &raw) };
        Ok(())
    }

    /// Send an XML metadata frame.
    pub fn send_metadata(&mut self, xml: &str) -> Result<()> {
        let text: CString = c_string(xml, "the NDI metadata")?;
        let raw = ffi::MetadataFrame {
            length: (xml.len() + 1) as i32,
            timecode: ffi::SEND_TIMECODE_SYNTHESIZE,
            p_data: text.as_ptr() as *mut _,
        };
        // SAFETY: the string is held for the duration of the call.
        unsafe { (self.lib.send_send_metadata)(self.instance, &raw) };
        Ok(())
    }
}

impl Drop for Sender {
    fn drop(&mut self) {
        // SAFETY: created by `send_create`, destroyed once; every send was
        // synchronous, so the runtime holds no buffer of ours.
        unsafe { (self.lib.send_destroy)(self.instance) }
    }
}
