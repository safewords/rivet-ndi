//! The NDI C ABI, declared by hand from the NDI SDK's public headers
//! (`Processing.NDI.structs.h`, `.Find.h`, `.Recv.h`, `.Send.h`,
//! `.Lib.h`), and the runtime library that provides it, opened with
//! `dlopen` / `LoadLibrary` when first asked for.
//!
//! Only what rivet calls is declared. Every struct is `#[repr(C)]` in the
//! headers' field order, and every enum a C `int`. The `v2` frame structs
//! have been stable since NDI 3 and are what the NDI 4, 5 and 6 runtimes all
//! export, so one declaration serves every runtime in the field.

use std::ffi::{c_char, c_float, c_int, c_void};
use std::path::{Path, PathBuf};

use crate::{Error, Result};

/// `NDI_LIB_FOURCC(a, b, c, d)`: the four bytes, first in the low byte.
pub const fn fourcc(code: &[u8; 4]) -> u32 {
    (code[0] as u32) | ((code[1] as u32) << 8) | ((code[2] as u32) << 16) | ((code[3] as u32) << 24)
}

// ─── Enums (C `int`) ───────────────────────────────────────────────

/// `NDIlib_frame_type_e` (0 is none: nothing arrived).
pub type FrameType = c_int;
pub const FRAME_TYPE_VIDEO: FrameType = 1;
pub const FRAME_TYPE_AUDIO: FrameType = 2;
pub const FRAME_TYPE_METADATA: FrameType = 3;
pub const FRAME_TYPE_ERROR: FrameType = 4;
/// The connection's settings changed (tally, PTZ, ...): nothing to read.
pub const FRAME_TYPE_STATUS_CHANGE: FrameType = 100;
/// NDI 6: the source the receiver is bound to changed (a new sender behind
/// the same name).
pub const FRAME_TYPE_SOURCE_CHANGE: FrameType = 101;

/// `NDIlib_frame_format_type_e` (0 interleaved, 2 and 3 single fields).
pub type FrameFormat = c_int;
pub const FRAME_FORMAT_PROGRESSIVE: FrameFormat = 1;

/// `NDIlib_recv_color_format_e`.
pub type RecvColorFormat = c_int;
pub const RECV_COLOR_FORMAT_UYVY_BGRA: RecvColorFormat = 1;
pub const RECV_COLOR_FORMAT_FASTEST: RecvColorFormat = 100;
pub const RECV_COLOR_FORMAT_BEST: RecvColorFormat = 101;

/// `NDIlib_recv_bandwidth_e`.
pub type RecvBandwidth = c_int;
pub const RECV_BANDWIDTH_METADATA_ONLY: RecvBandwidth = -10;
pub const RECV_BANDWIDTH_AUDIO_ONLY: RecvBandwidth = 10;
pub const RECV_BANDWIDTH_LOWEST: RecvBandwidth = 0;
pub const RECV_BANDWIDTH_HIGHEST: RecvBandwidth = 100;

/// `NDIlib_send_timecode_synthesize`: let the SDK stamp the timecode.
pub const SEND_TIMECODE_SYNTHESIZE: i64 = i64::MAX;
/// `NDIlib_recv_timestamp_undefined`: the sender predates timestamps.
pub const RECV_TIMESTAMP_UNDEFINED: i64 = i64::MAX;

// ─── Structs ───────────────────────────────────────────────────────

/// `NDIlib_source_t`. The second member is a union of `p_url_address` and
/// `p_ip_address`; both are a `const char*`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Source {
    pub p_ndi_name: *const c_char,
    pub p_url_address: *const c_char,
}

/// `NDIlib_find_create_t`.
#[repr(C)]
pub struct FindCreate {
    pub show_local_sources: bool,
    pub p_groups: *const c_char,
    pub p_extra_ips: *const c_char,
}

/// `NDIlib_recv_create_v3_t`.
#[repr(C)]
pub struct RecvCreateV3 {
    pub source_to_connect_to: Source,
    pub color_format: RecvColorFormat,
    pub bandwidth: RecvBandwidth,
    pub allow_video_fields: bool,
    pub p_ndi_recv_name: *const c_char,
}

/// `NDIlib_send_create_t`.
#[repr(C)]
pub struct SendCreate {
    pub p_ndi_name: *const c_char,
    pub p_groups: *const c_char,
    pub clock_video: bool,
    pub clock_audio: bool,
}

/// `NDIlib_video_frame_v2_t`. `line_stride_in_bytes` is a union with
/// `data_size_in_bytes` (the latter for compressed fourccs, which rivet
/// neither asks for nor sends).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct VideoFrameV2 {
    pub xres: c_int,
    pub yres: c_int,
    pub fourcc: u32,
    pub frame_rate_n: c_int,
    pub frame_rate_d: c_int,
    pub picture_aspect_ratio: c_float,
    pub frame_format_type: FrameFormat,
    pub timecode: i64,
    pub p_data: *mut u8,
    pub line_stride_in_bytes: c_int,
    pub p_metadata: *const c_char,
    pub timestamp: i64,
}

impl VideoFrameV2 {
    pub fn zeroed() -> Self {
        Self {
            xres: 0,
            yres: 0,
            fourcc: 0,
            frame_rate_n: 0,
            frame_rate_d: 0,
            picture_aspect_ratio: 0.0,
            frame_format_type: FRAME_FORMAT_PROGRESSIVE,
            timecode: 0,
            p_data: std::ptr::null_mut(),
            line_stride_in_bytes: 0,
            p_metadata: std::ptr::null(),
            timestamp: 0,
        }
    }
}

/// `NDIlib_audio_frame_v2_t`: 32-bit float, planar, one channel every
/// `channel_stride_in_bytes`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct AudioFrameV2 {
    pub sample_rate: c_int,
    pub no_channels: c_int,
    pub no_samples: c_int,
    pub timecode: i64,
    pub p_data: *mut c_float,
    pub channel_stride_in_bytes: c_int,
    pub p_metadata: *const c_char,
    pub timestamp: i64,
}

impl AudioFrameV2 {
    pub fn zeroed() -> Self {
        Self {
            sample_rate: 0,
            no_channels: 0,
            no_samples: 0,
            timecode: 0,
            p_data: std::ptr::null_mut(),
            channel_stride_in_bytes: 0,
            p_metadata: std::ptr::null(),
            timestamp: 0,
        }
    }
}

/// `NDIlib_metadata_frame_t`: a null-terminated UTF-8 XML string.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct MetadataFrame {
    pub length: c_int,
    pub timecode: i64,
    pub p_data: *mut c_char,
}

impl MetadataFrame {
    pub fn zeroed() -> Self {
        Self {
            length: 0,
            timecode: 0,
            p_data: std::ptr::null_mut(),
        }
    }
}

// ─── Entry points ─────────────────────────────────────────────────

pub type Instance = *mut c_void;

/// The runtime's exports rivet calls, resolved once. The `Library` is kept
/// last so it outlives every pointer above it.
pub struct Lib {
    pub initialize: unsafe extern "C" fn() -> bool,
    pub version: unsafe extern "C" fn() -> *const c_char,
    pub is_supported_cpu: unsafe extern "C" fn() -> bool,

    pub find_create_v2: unsafe extern "C" fn(*const FindCreate) -> Instance,
    pub find_destroy: unsafe extern "C" fn(Instance),
    pub find_get_current_sources: unsafe extern "C" fn(Instance, *mut u32) -> *const Source,
    pub find_wait_for_sources: unsafe extern "C" fn(Instance, u32) -> bool,

    pub recv_create_v3: unsafe extern "C" fn(*const RecvCreateV3) -> Instance,
    pub recv_destroy: unsafe extern "C" fn(Instance),
    pub recv_capture_v2: unsafe extern "C" fn(
        Instance,
        *mut VideoFrameV2,
        *mut AudioFrameV2,
        *mut MetadataFrame,
        u32,
    ) -> FrameType,
    pub recv_free_video_v2: unsafe extern "C" fn(Instance, *const VideoFrameV2),
    pub recv_free_audio_v2: unsafe extern "C" fn(Instance, *const AudioFrameV2),
    pub recv_free_metadata: unsafe extern "C" fn(Instance, *const MetadataFrame),
    pub recv_get_no_connections: unsafe extern "C" fn(Instance) -> c_int,

    pub send_create: unsafe extern "C" fn(*const SendCreate) -> Instance,
    pub send_destroy: unsafe extern "C" fn(Instance),
    pub send_send_video_v2: unsafe extern "C" fn(Instance, *const VideoFrameV2),
    pub send_send_audio_v2: unsafe extern "C" fn(Instance, *const AudioFrameV2),
    pub send_send_metadata: unsafe extern "C" fn(Instance, *const MetadataFrame),
    pub send_get_no_connections: unsafe extern "C" fn(Instance, u32) -> c_int,

    /// Where the runtime was loaded from, for messages.
    pub path: PathBuf,
    _library: libloading::Library,
}

// The function pointers are plain code addresses and the SDK is documented
// thread-safe across instances; the `Library` is only held, never used.
unsafe impl Send for Lib {}
unsafe impl Sync for Lib {}

/// The runtime's file name on this platform, for messages.
pub const LIBRARY_NAME: &str = if cfg!(windows) {
    "Processing.NDI.Lib.x64.dll"
} else if cfg!(target_os = "macos") {
    "libndi.dylib"
} else {
    "libndi.so.6"
};

/// The runtime file names to try, most specific first: `RIVET_NDI_LIB` (a
/// full path), then each `NDI_RUNTIME_DIR_V*` the NDI installers set, then
/// the bare name for the platform loader's own search (`PATH`, the
/// `ld.so` cache, `/usr/local/lib`).
pub fn candidates() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(p) = std::env::var_os("RIVET_NDI_LIB") {
        out.push(PathBuf::from(p));
    }
    let (file, bare): (&str, &[&str]) = if cfg!(windows) {
        if cfg!(target_pointer_width = "64") {
            (
                "Processing.NDI.Lib.x64.dll",
                &["Processing.NDI.Lib.x64.dll"],
            )
        } else {
            (
                "Processing.NDI.Lib.x86.dll",
                &["Processing.NDI.Lib.x86.dll"],
            )
        }
    } else if cfg!(target_os = "macos") {
        (
            "libndi.dylib",
            &["libndi.dylib", "/usr/local/lib/libndi.dylib"],
        )
    } else {
        (
            "libndi.so.6",
            &["libndi.so.6", "libndi.so.5", "libndi.so", "libndi.so.4"],
        )
    };
    for var in [
        "NDI_RUNTIME_DIR_V6",
        "NDI_RUNTIME_DIR_V5",
        "NDI_RUNTIME_DIR_V4",
    ] {
        if let Some(dir) = std::env::var_os(var) {
            out.push(Path::new(&dir).join(file));
        }
    }
    if cfg!(windows) {
        // The NDI 6 runtime and NDI Tools installers' default directories,
        // for a shell started before the installer set the variable.
        for dir in [
            r"C:\Program Files\NDI\NDI 6 Runtime\v6",
            r"C:\Program Files\NDI\NDI 6 Tools\Runtime",
            r"C:\Program Files\NDI\NDI 5 Runtime\v5",
        ] {
            out.push(Path::new(dir).join(file));
        }
    }
    out.extend(bare.iter().map(PathBuf::from));
    out
}

impl Lib {
    /// Open the first runtime [`candidates`] names and resolve every export.
    pub fn open() -> Result<Self> {
        let mut tried = Vec::new();
        for path in candidates() {
            // A full path that is not there is not worth a loader error.
            if path.components().count() > 1 && !path.exists() {
                tried.push(format!("{} (not found)", path.display()));
                continue;
            }
            // SAFETY: loading the NDI runtime runs its initialisers, which
            // is what the SDK expects of every host that uses it.
            match unsafe { libloading::Library::new(&path) } {
                Ok(lib) => return Self::resolve(lib, path),
                Err(e) => tried.push(format!("{} ({e})", path.display())),
            }
        }
        Err(Error::RuntimeNotFound {
            tried: tried.join("; "),
        })
    }

    fn resolve(library: libloading::Library, path: PathBuf) -> Result<Self> {
        macro_rules! sym {
            ($name:literal) => {{
                // SAFETY: the type is the header's prototype for the export.
                let s = unsafe { library.get(concat!($name, "\0").as_bytes()) }.map_err(|_| {
                    Error::MissingExport {
                        path: path.clone(),
                        name: $name,
                    }
                })?;
                *s
            }};
        }
        Ok(Self {
            initialize: sym!("NDIlib_initialize"),
            version: sym!("NDIlib_version"),
            is_supported_cpu: sym!("NDIlib_is_supported_CPU"),
            find_create_v2: sym!("NDIlib_find_create_v2"),
            find_destroy: sym!("NDIlib_find_destroy"),
            find_get_current_sources: sym!("NDIlib_find_get_current_sources"),
            find_wait_for_sources: sym!("NDIlib_find_wait_for_sources"),
            recv_create_v3: sym!("NDIlib_recv_create_v3"),
            recv_destroy: sym!("NDIlib_recv_destroy"),
            recv_capture_v2: sym!("NDIlib_recv_capture_v2"),
            recv_free_video_v2: sym!("NDIlib_recv_free_video_v2"),
            recv_free_audio_v2: sym!("NDIlib_recv_free_audio_v2"),
            recv_free_metadata: sym!("NDIlib_recv_free_metadata"),
            recv_get_no_connections: sym!("NDIlib_recv_get_no_connections"),
            send_create: sym!("NDIlib_send_create"),
            send_destroy: sym!("NDIlib_send_destroy"),
            send_send_video_v2: sym!("NDIlib_send_send_video_v2"),
            send_send_audio_v2: sym!("NDIlib_send_send_audio_v2"),
            send_send_metadata: sym!("NDIlib_send_send_metadata"),
            send_get_no_connections: sym!("NDIlib_send_get_no_connections"),
            path,
            _library: library,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The structs are laid out as the headers lay them out on a 64-bit
    /// target: a field the runtime reads at the wrong offset is a picture
    /// of the wrong size or a pointer into nowhere.
    #[test]
    #[cfg(target_pointer_width = "64")]
    fn struct_layouts_match_the_sdk_headers() {
        use std::mem::{offset_of, size_of};
        assert_eq!(size_of::<Source>(), 16);
        assert_eq!(size_of::<FindCreate>(), 24);
        assert_eq!(size_of::<RecvCreateV3>(), 40);
        assert_eq!(offset_of!(RecvCreateV3, color_format), 16);
        assert_eq!(offset_of!(RecvCreateV3, p_ndi_recv_name), 32);
        assert_eq!(size_of::<SendCreate>(), 24);
        assert_eq!(offset_of!(SendCreate, clock_audio), 17);

        assert_eq!(offset_of!(VideoFrameV2, fourcc), 8);
        assert_eq!(offset_of!(VideoFrameV2, frame_format_type), 24);
        assert_eq!(offset_of!(VideoFrameV2, timecode), 32);
        assert_eq!(offset_of!(VideoFrameV2, p_data), 40);
        assert_eq!(offset_of!(VideoFrameV2, line_stride_in_bytes), 48);
        assert_eq!(offset_of!(VideoFrameV2, p_metadata), 56);
        assert_eq!(offset_of!(VideoFrameV2, timestamp), 64);
        assert_eq!(size_of::<VideoFrameV2>(), 72);

        assert_eq!(offset_of!(AudioFrameV2, timecode), 16);
        assert_eq!(offset_of!(AudioFrameV2, p_data), 24);
        assert_eq!(offset_of!(AudioFrameV2, channel_stride_in_bytes), 32);
        assert_eq!(offset_of!(AudioFrameV2, p_metadata), 40);
        assert_eq!(offset_of!(AudioFrameV2, timestamp), 48);
        assert_eq!(size_of::<AudioFrameV2>(), 56);

        assert_eq!(size_of::<MetadataFrame>(), 24);
    }

    #[test]
    fn fourcc_puts_the_first_byte_low() {
        assert_eq!(fourcc(b"UYVY"), 0x5956_5955);
        assert_eq!(fourcc(b"I420"), u32::from_le_bytes(*b"I420"));
    }

    #[test]
    fn rivet_ndi_lib_is_tried_first() {
        // Reading the variable is enough: candidates() takes it verbatim.
        if let Some(p) = std::env::var_os("RIVET_NDI_LIB") {
            assert_eq!(candidates()[0], PathBuf::from(p));
        } else {
            assert!(!candidates().is_empty());
        }
    }
}
