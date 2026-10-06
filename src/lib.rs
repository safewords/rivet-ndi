//! # rivet-ndi
//!
//! [NDI®](https://ndi.video) source discovery, receive and send, through FFI
//! written by hand against the NDI SDK's public C headers. The NDI runtime
//! is loaded when first asked for (`dlopen` / `LoadLibrary`), so nothing
//! about NDI is needed to *build* — no SDK, no bindgen, no link step, no
//! build script — and a host without the runtime gets
//! [`Error::RuntimeNotFound`] rather than a binary that will not start.
//!
//! ```no_run
//! use std::time::Duration;
//!
//! let ndi = ndi::Ndi::load()?;
//! for source in ndi.sources(&ndi::FindOptions::default(), Duration::from_secs(2))? {
//!     println!("{}", source.name);
//! }
//!
//! let source = ndi.find_source("Camera 1", &ndi::FindOptions::default(), Duration::from_secs(5))?;
//! let mut receiver = ndi.receiver(&source, &ndi::ReceiverOptions::default())?;
//! loop {
//!     match receiver.capture(Duration::from_millis(500))? {
//!         ndi::Capture::Video(frame) => {
//!             let picture = frame.to_picture()?; // planar, tightly packed
//!             println!("{}x{} {:?}", picture.width, picture.height, picture.layout);
//!         }
//!         ndi::Capture::Audio(audio) => println!("{} samples", audio.samples()),
//!         _ => {}
//!     }
//! }
//! # Ok::<(), ndi::Error>(())
//! ```
//!
//! Where the runtime is looked for: `RIVET_NDI_LIB` (a full path to the
//! library), then the directories the NDI installers name in
//! `NDI_RUNTIME_DIR_V6` / `_V5` / `_V4`, then the platform loader's own
//! search for `Processing.NDI.Lib.x64.dll` (Windows) or `libndi.so.6`
//! (Linux). NDI 4 or later.
//!
//! NDI® is a registered trademark of Vizrt NDI AB. This crate is not
//! affiliated with or endorsed by Vizrt; it redistributes no part of the
//! NDI SDK or runtime.

pub mod convert;
mod ffi;
mod find;
mod recv;
mod send;

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

pub use convert::{ColorInfo, FourCC, Layout, Picture};
pub use find::{FindOptions, Finder, Source};
pub use recv::{
    AudioFrame, Bandwidth, Capture, ColorFormat, Receiver, ReceiverOptions, VideoFrame,
};
pub use send::{OutgoingVideo, Sender, SenderOptions};

/// Everything that can go wrong here.
#[derive(Debug, Clone, thiserror::Error)]
pub enum Error {
    #[error(
        "no NDI runtime found: install the NDI runtime (https://ndi.video/tools/ — NDI Tools, or \
         the stand-alone runtime), or point RIVET_NDI_LIB at the library ({}). Tried: {tried}",
        ffi::LIBRARY_NAME
    )]
    RuntimeNotFound { tried: String },
    #[error(
        "the NDI runtime at {} has no `{name}` export: NDI 4 or later is needed",
        path.display()
    )]
    MissingExport { path: PathBuf, name: &'static str },
    #[error(
        "the NDI runtime at {} would not initialise (NDIlib_initialize failed{})",
        path.display(),
        if *cpu_supported { "" } else { ": this CPU lacks the SSE4 instructions NDI needs" }
    )]
    InitFailed { path: PathBuf, cpu_supported: bool },
    #[error("{0} failed")]
    CreateFailed(&'static str),
    #[error("{what} holds a NUL byte")]
    Nul { what: &'static str },
    #[error("no NDI source matching `{query}` appeared within {waited_ms} ms (seen: {seen})")]
    SourceNotFound {
        query: String,
        waited_ms: u128,
        seen: String,
    },
    #[error("`{query}` matches several NDI sources ({matches}); name one in full")]
    AmbiguousSource { query: String, matches: String },
    #[error("NDI video in {0} is not a layout this crate reads")]
    UnsupportedFourCC(FourCC),
    #[error("bad NDI frame: {0}")]
    BadFrame(String),
    #[error("the NDI receiver reported an error: the connection was lost")]
    ConnectionLost,
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

/// The loaded NDI runtime. Cheap to clone; the library stays loaded for
/// the life of the process once opened (the SDK is initialised once and
/// never torn down, which is what it recommends for a host that may use
/// it again).
#[derive(Clone)]
pub struct Ndi {
    lib: Arc<ffi::Lib>,
}

static RUNTIME: OnceLock<Result<Arc<ffi::Lib>>> = OnceLock::new();

impl Ndi {
    /// Load and initialise the runtime, or hand back the one already loaded.
    /// A failure is remembered: the search is not repeated.
    pub fn load() -> Result<Self> {
        RUNTIME
            .get_or_init(|| {
                let lib = ffi::Lib::open()?;
                // SAFETY: plain calls into the loaded runtime.
                let cpu_supported = unsafe { (lib.is_supported_cpu)() };
                if !unsafe { (lib.initialize)() } {
                    return Err(Error::InitFailed {
                        path: lib.path.clone(),
                        cpu_supported,
                    });
                }
                Ok(Arc::new(lib))
            })
            .clone()
            .map(|lib| Self { lib })
    }

    /// Whether a runtime can be loaded here, without failing.
    pub fn is_available() -> bool {
        Self::load().is_ok()
    }

    /// The runtime's version string (`NDIlib_version`), e.g.
    /// `NDI SDK WIN64 ... 6.1.1.0`.
    pub fn version(&self) -> String {
        // SAFETY: a static NUL-terminated string owned by the runtime.
        unsafe { cstr_to_string((self.lib.version)()) }.unwrap_or_default()
    }

    /// The file the runtime was loaded from.
    pub fn path(&self) -> &Path {
        &self.lib.path
    }

    /// A finder: discovery runs in the background while it lives.
    pub fn finder(&self, options: &FindOptions) -> Result<Finder> {
        Finder::new(Arc::clone(&self.lib), options)
    }

    /// Every source seen within `wait`.
    pub fn sources(&self, options: &FindOptions, wait: std::time::Duration) -> Result<Vec<Source>> {
        self.finder(options)?.sources_within(wait)
    }

    /// The one source `query` names, waiting up to `wait` for it to appear:
    /// an exact name first (case-insensitive), else the only name that
    /// contains it. NDI names are `MACHINE (Stream)`, so `Stream` or
    /// `MACHINE` alone finds a source when it is the only one of its kind.
    pub fn find_source(
        &self,
        query: &str,
        options: &FindOptions,
        wait: std::time::Duration,
    ) -> Result<Source> {
        self.finder(options)?.find(query, wait)
    }

    /// Connect a receiver to `source`.
    pub fn receiver(&self, source: &Source, options: &ReceiverOptions) -> Result<Receiver> {
        Receiver::new(Arc::clone(&self.lib), source, options)
    }

    /// Announce a source on the network.
    pub fn sender(&self, options: &SenderOptions) -> Result<Sender> {
        Sender::new(Arc::clone(&self.lib), options)
    }
}

/// A runtime-owned C string, copied out. `None` for a null pointer.
///
/// # Safety
/// `p` is null or points at a NUL-terminated string that lives for the call.
unsafe fn cstr_to_string(p: *const std::ffi::c_char) -> Option<String> {
    if p.is_null() {
        None
    } else {
        Some(
            unsafe { std::ffi::CStr::from_ptr(p) }
                .to_string_lossy()
                .into_owned(),
        )
    }
}

/// `s` as a C string, or the error naming `what` holds a NUL.
fn c_string(s: &str, what: &'static str) -> Result<std::ffi::CString> {
    std::ffi::CString::new(s).map_err(|_| Error::Nul { what })
}

/// A timestamp the SDK hands back, `None` when it is the "undefined" marker
/// or zero (a sender that stamps nothing).
fn defined_timestamp(t: i64) -> Option<i64> {
    (t != ffi::RECV_TIMESTAMP_UNDEFINED && t != 0).then_some(t)
}
