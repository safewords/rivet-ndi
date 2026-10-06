# rivet-ndi

[![CI](https://github.com/safewords/rivet-ndi/actions/workflows/ci.yml/badge.svg)](https://github.com/safewords/rivet-ndi/actions/workflows/ci.yml)

[NDI®](https://ndi.video) source discovery, receive and send in Rust,
through FFI written by hand against the NDI SDK's public C headers. The NDI
runtime is loaded the first time it is asked for (`LoadLibrary` / `dlopen`),
so nothing about NDI is needed to **build**: no SDK, no bindgen, no link
step, no build script. A host without the runtime gets an error saying how
to install it, not a binary that will not start.

Written for the **[rivet](https://github.com/safewords/rivet)** transcoder,
where it is the `ndi` feature: `rivet ndi record` encodes a live NDI source
into a file and `rivet ndi send` plays a file out as one (see rivet's
[docs/ndi.md](https://github.com/safewords/rivet/blob/develop/docs/ndi.md)).
Usable on its own by anything that wants NDI pictures and sound in, or out.

Published as `rivet-ndi`; **imported as `ndi`** (`use ndi::…`). Two
dependencies (`libloading`, `thiserror`), no features, no build script.

```toml
[dependencies]
ndi = { package = "rivet-ndi", git = "https://github.com/safewords/rivet-ndi", branch = "develop" }
```

## Use

```rust,no_run
use std::time::Duration;

let ndi = ndi::Ndi::load()?;
println!("{}", ndi.version());

// Discovery.
for source in ndi.sources(&ndi::FindOptions::default(), Duration::from_secs(2))? {
    println!("{}  {:?}", source.name, source.url);
}

// Receive: the one source a name (or a unique part of one) names.
let source = ndi.find_source("Camera 1", &ndi::FindOptions::default(), Duration::from_secs(5))?;
let mut receiver = ndi.receiver(&source, &ndi::ReceiverOptions::default())?;
match receiver.capture(Duration::from_millis(500))? {
    ndi::Capture::Video(frame) => {
        let picture = frame.to_picture()?; // planar, no row padding
        println!("{}x{} {:?} at {:?}", picture.width, picture.height, picture.layout, frame.frame_rate());
    }
    ndi::Capture::Audio(audio) => {
        let interleaved = audio.interleaved(); // f32, L R L R ...
        println!("{} samples x {} channels", audio.samples(), audio.channels());
    }
    _ => {}
}

// Send: announce a source, paced by the runtime at the frame rate given.
let mut sender = ndi.sender(&ndi::SenderOptions::new("My output"))?;
let picture = ndi::Picture {
    layout: ndi::Layout::Yuv420p,
    width: 1280,
    height: 720,
    data: vec![128; 1280 * 720 * 3 / 2],
};
sender.send_picture(&picture, (30000, 1001), None)?;
sender.send_audio(48_000, 2, &vec![0.0; 1601 * 2], None)?;
# Ok::<(), ndi::Error>(())
```

Frames are the runtime's until dropped: a received `VideoFrame` or
`AudioFrame` hands its buffer back to the runtime on drop, and borrows
nothing, so it can be moved to another thread first.

## Pictures

NDI carries video uncompressed in a few fourccs, rows `stride` bytes apart.
`VideoFrame::to_picture` (or `convert::to_picture` on raw bytes) packs them
into a planar `Picture`; `Picture::to_ndi` goes the other way:

| NDI fourcc | `Picture` layout | Back out as |
|---|---|---|
| UYVY, UYVA (alpha dropped) | `Yuv422p` | UYVY |
| P216, PA16 (alpha dropped) | `Yuv422p10le` (16 → 10 bits, rounded) | P216 |
| I420, YV12 | `Yuv420p` | I420 |
| NV12 | `Nv12` | NV12 |
| BGRA, BGRX, RGBA, RGBX | `Rgba` (X opaque) | RGBA |
| — | `Yuv420p10le` | P216 (chroma rows repeated) |

Only samples move: nothing is re-matrixed. NDI's YUV is studio range,
BT.601 below 720 lines and BT.709 from 720 up, unless the frame's metadata
says otherwise — `VideoFrame::color_info` reads NDI 6's
`<ndi_color_info transfer=… matrix=… primaries=…/>`. Odd widths and heights
are handled (chroma `(n + 1) / 2`); a short buffer or stride is an error,
never a panic.

## The runtime

Looked for, in order: `RIVET_NDI_LIB` (a full path), the directories the
NDI installers set in `NDI_RUNTIME_DIR_V6` / `_V5` / `_V4`, the installers'
default directories on Windows, then the platform loader's own search for
`Processing.NDI.Lib.x64.dll` (Windows) or `libndi.so.6` / `.5` (Linux).
NDI 4 or later; the `v2` frame structs every runtime since NDI 3 exports
are the ones declared.

The runtime is Vizrt's, installed by the user under NDI's licence
([ndi.video/tools](https://ndi.video/tools/)); this crate redistributes no
part of the NDI SDK or runtime.

## How it is checked

`cargo test`: every struct's size and field offsets against the SDK
headers' 64-bit layout, each fourcc conversion (padded strides, odd sizes,
16 → 10-bit rounding, round trips), colour metadata parsing, and source-name
matching. No runtime is needed. rivet's `ndi_loopback` test goes through a
real runtime: a sender and a receiver on one machine, pictures and a tone,
recorded and checked.

## Licence

Source-available under the Open Encoding Attribution License 1.0
([LICENSE.md](LICENSE.md)); see [NOTICE](NOTICE).

NDI® is a registered trademark of Vizrt NDI AB. This crate is not affiliated
with or endorsed by Vizrt.
