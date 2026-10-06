//! NDI's pixel layouts to and from tightly packed planar pictures.
//!
//! NDI carries video uncompressed in one of a handful of fourccs, each row
//! `stride` bytes apart. A [`Picture`] is the same pixels with the planes
//! one after another and no padding — the layout decoders hand a
//! transcoder, so a received frame goes straight into colour conversion and
//! an encoder, and a decoded one straight out over NDI.
//!
//! The conversions only move samples. Nothing is re-matrixed or re-ranged:
//! NDI's YUV is studio range, BT.601 below 720 lines and BT.709 from 720 up
//! unless the frame's metadata says otherwise ([`ColorInfo`]), and its RGB
//! is full-range sRGB.

use crate::ffi::fourcc;
use crate::{Error, Result};

/// An NDI video fourcc.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FourCC {
    /// 8-bit 4:2:2, `U0 Y0 V0 Y1` per two pixels.
    Uyvy,
    /// `Uyvy`, then an 8-bit alpha plane.
    Uyva,
    /// 16-bit 4:2:2, semi-planar: a Y plane, then an interleaved `U V` plane
    /// of the same height.
    P216,
    /// `P216`, then a 16-bit alpha plane.
    Pa16,
    /// 8-bit 4:2:0 planar: Y, then V, then U.
    Yv12,
    /// 8-bit 4:2:0 planar: Y, then U, then V.
    I420,
    /// 8-bit 4:2:0: Y, then interleaved `U V`.
    Nv12,
    Bgra,
    Bgrx,
    Rgba,
    Rgbx,
    /// Anything else (a compressed or future fourcc).
    Other(u32),
}

impl FourCC {
    pub fn from_raw(raw: u32) -> Self {
        const UYVY: u32 = fourcc(b"UYVY");
        const UYVA: u32 = fourcc(b"UYVA");
        const P216: u32 = fourcc(b"P216");
        const PA16: u32 = fourcc(b"PA16");
        const YV12: u32 = fourcc(b"YV12");
        const I420: u32 = fourcc(b"I420");
        const NV12: u32 = fourcc(b"NV12");
        const BGRA: u32 = fourcc(b"BGRA");
        const BGRX: u32 = fourcc(b"BGRX");
        const RGBA: u32 = fourcc(b"RGBA");
        const RGBX: u32 = fourcc(b"RGBX");
        match raw {
            UYVY => Self::Uyvy,
            UYVA => Self::Uyva,
            P216 => Self::P216,
            PA16 => Self::Pa16,
            YV12 => Self::Yv12,
            I420 => Self::I420,
            NV12 => Self::Nv12,
            BGRA => Self::Bgra,
            BGRX => Self::Bgrx,
            RGBA => Self::Rgba,
            RGBX => Self::Rgbx,
            other => Self::Other(other),
        }
    }

    pub fn raw(self) -> u32 {
        match self {
            Self::Uyvy => fourcc(b"UYVY"),
            Self::Uyva => fourcc(b"UYVA"),
            Self::P216 => fourcc(b"P216"),
            Self::Pa16 => fourcc(b"PA16"),
            Self::Yv12 => fourcc(b"YV12"),
            Self::I420 => fourcc(b"I420"),
            Self::Nv12 => fourcc(b"NV12"),
            Self::Bgra => fourcc(b"BGRA"),
            Self::Bgrx => fourcc(b"BGRX"),
            Self::Rgba => fourcc(b"RGBA"),
            Self::Rgbx => fourcc(b"RGBX"),
            Self::Other(raw) => raw,
        }
    }

    /// Bytes a frame of this fourcc occupies with rows `stride` apart (the
    /// stride of the first plane; the chroma planes of the planar 4:2:0
    /// layouts are half of it). `None` for a fourcc this crate does not read.
    pub fn buffer_len(self, width: usize, height: usize, stride: usize) -> Option<usize> {
        let half_h = height.div_ceil(2);
        Some(match self {
            Self::Uyvy | Self::Bgra | Self::Bgrx | Self::Rgba | Self::Rgbx => stride * height,
            Self::Uyva => stride * height + width * height,
            Self::P216 => stride * height * 2,
            Self::Pa16 => stride * height * 3,
            Self::Yv12 | Self::I420 => stride * height + 2 * stride.div_ceil(2) * half_h,
            Self::Nv12 => stride * height + stride * half_h,
            Self::Other(_) => return None,
        })
    }
}

impl std::fmt::Display for FourCC {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let raw = self.raw().to_le_bytes();
        if raw.iter().all(|b| b.is_ascii_graphic() || *b == b' ') {
            write!(f, "{}", String::from_utf8_lossy(&raw))
        } else {
            write!(f, "0x{:08x}", self.raw())
        }
    }
}

/// The packed layouts a [`Picture`] is in. Chroma planes are
/// `(width + 1) / 2` wide, and for 4:2:0 `(height + 1) / 2` tall.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Layout {
    /// 8-bit 4:2:0: Y, U, V planes.
    Yuv420p,
    /// 10-bit 4:2:0: Y, U, V planes of little-endian `u16` in 0..=1023.
    Yuv420p10le,
    /// 8-bit 4:2:2: Y, U, V planes.
    Yuv422p,
    /// 10-bit 4:2:2: Y, U, V planes of little-endian `u16` in 0..=1023.
    Yuv422p10le,
    /// 8-bit 4:2:0: a Y plane, then interleaved `U V`.
    Nv12,
    /// 8-bit `R G B A`, full range.
    Rgba,
}

impl Layout {
    /// Bytes a `width` x `height` picture of this layout occupies.
    pub fn len(self, width: usize, height: usize) -> usize {
        let (cw, ch) = (width.div_ceil(2), height.div_ceil(2));
        match self {
            Self::Yuv420p => width * height + 2 * cw * ch,
            Self::Yuv420p10le => 2 * (width * height + 2 * cw * ch),
            Self::Yuv422p => width * height + 2 * cw * height,
            Self::Yuv422p10le => 2 * (width * height + 2 * cw * height),
            Self::Nv12 => width * height + 2 * cw * ch,
            Self::Rgba => 4 * width * height,
        }
    }
}

/// A picture with its planes packed one after another, no row padding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Picture {
    pub layout: Layout,
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}

/// An NDI frame's pixels, rows `stride` bytes apart, as a [`Picture`]:
/// UYVY / UYVA → `Yuv422p` (the alpha dropped), P216 / PA16 → `Yuv422p10le`
/// (rounded from 16 to 10 bits), I420 / YV12 → `Yuv420p`, NV12 → `Nv12`, and
/// the four RGB orders → `Rgba` (BGRX / RGBX opaque).
pub fn to_picture(
    fourcc: FourCC,
    width: u32,
    height: u32,
    stride: usize,
    data: &[u8],
) -> Result<Picture> {
    let (w, h) = (width as usize, height as usize);
    if w == 0 || h == 0 {
        return Err(Error::BadFrame(format!("{width}x{height} picture")));
    }
    let need = fourcc
        .buffer_len(w, h, stride)
        .ok_or(Error::UnsupportedFourCC(fourcc))?;
    let min_stride = match fourcc {
        FourCC::Uyvy | FourCC::Uyva => w.div_ceil(2) * 4,
        FourCC::P216 | FourCC::Pa16 => w.div_ceil(2) * 4,
        FourCC::Yv12 | FourCC::I420 | FourCC::Nv12 => w,
        _ => w * 4,
    };
    if stride < min_stride || data.len() < need {
        return Err(Error::BadFrame(format!(
            "{fourcc} {width}x{height}: stride {stride} (at least {min_stride}), {} bytes (at least {need})",
            data.len()
        )));
    }
    let (cw, ch) = (w.div_ceil(2), h.div_ceil(2));
    let rows = |plane: &[u8], stride: usize, row_len: usize, n: usize, out: &mut Vec<u8>| {
        for r in 0..n {
            out.extend_from_slice(&plane[r * stride..r * stride + row_len]);
        }
    };
    let (layout, out) = match fourcc {
        FourCC::Uyvy | FourCC::Uyva => {
            let mut y = Vec::with_capacity(w * h);
            let mut u = Vec::with_capacity(cw * h);
            let mut v = Vec::with_capacity(cw * h);
            for r in 0..h {
                let row = &data[r * stride..r * stride + cw * 4];
                for (i, quad) in row.as_chunks::<4>().0.iter().enumerate() {
                    u.push(quad[0]);
                    y.push(quad[1]);
                    v.push(quad[2]);
                    if 2 * i + 1 < w {
                        y.push(quad[3]);
                    }
                }
            }
            y.extend_from_slice(&u);
            y.extend_from_slice(&v);
            (Layout::Yuv422p, y)
        }
        FourCC::P216 | FourCC::Pa16 => {
            let ten = |lo: u8, hi: u8| -> [u8; 2] {
                let v16 = u16::from_le_bytes([lo, hi]) as u32;
                (((v16 + 32) >> 6).min(1023) as u16).to_le_bytes()
            };
            let mut y = Vec::with_capacity(2 * w * h);
            let mut u = Vec::with_capacity(2 * cw * h);
            let mut v = Vec::with_capacity(2 * cw * h);
            for r in 0..h {
                let row = &data[r * stride..r * stride + 2 * w];
                for s in row.as_chunks::<2>().0.iter() {
                    y.extend_from_slice(&ten(s[0], s[1]));
                }
            }
            let uv = &data[stride * h..];
            for r in 0..h {
                let row = &uv[r * stride..r * stride + 4 * cw];
                for s in row.as_chunks::<4>().0.iter() {
                    u.extend_from_slice(&ten(s[0], s[1]));
                    v.extend_from_slice(&ten(s[2], s[3]));
                }
            }
            y.extend_from_slice(&u);
            y.extend_from_slice(&v);
            (Layout::Yuv422p10le, y)
        }
        FourCC::I420 | FourCC::Yv12 => {
            let cstride = stride.div_ceil(2);
            let mut out = Vec::with_capacity(Layout::Yuv420p.len(w, h));
            rows(data, stride, w, h, &mut out);
            let first = &data[stride * h..];
            let second = &first[cstride * ch..];
            let (u, v) = if fourcc == FourCC::I420 {
                (first, second)
            } else {
                (second, first)
            };
            rows(u, cstride, cw, ch, &mut out);
            rows(v, cstride, cw, ch, &mut out);
            (Layout::Yuv420p, out)
        }
        FourCC::Nv12 => {
            let mut out = Vec::with_capacity(Layout::Nv12.len(w, h));
            rows(data, stride, w, h, &mut out);
            rows(&data[stride * h..], stride, 2 * cw, ch, &mut out);
            (Layout::Nv12, out)
        }
        FourCC::Bgra | FourCC::Bgrx | FourCC::Rgba | FourCC::Rgbx => {
            let swap = matches!(fourcc, FourCC::Bgra | FourCC::Bgrx);
            let opaque = matches!(fourcc, FourCC::Bgrx | FourCC::Rgbx);
            let mut out = Vec::with_capacity(4 * w * h);
            for r in 0..h {
                for px in data[r * stride..r * stride + 4 * w]
                    .as_chunks::<4>()
                    .0
                    .iter()
                {
                    let (red, blue) = if swap { (px[2], px[0]) } else { (px[0], px[2]) };
                    out.extend_from_slice(&[red, px[1], blue, if opaque { 255 } else { px[3] }]);
                }
            }
            (Layout::Rgba, out)
        }
        FourCC::Other(_) => return Err(Error::UnsupportedFourCC(fourcc)),
    };
    Ok(Picture {
        layout,
        width,
        height,
        data: out,
    })
}

impl Picture {
    /// The NDI fourcc this picture goes out as, its row stride, and the
    /// bytes: `Yuv420p` as I420 and `Nv12` as NV12 (the same bytes),
    /// `Yuv422p` as UYVY, the 10-bit layouts as P216 (4:2:0 chroma repeated
    /// down to 4:2:2), `Rgba` as RGBA.
    pub fn to_ndi(&self) -> Result<(FourCC, usize, Vec<u8>)> {
        let (w, h) = (self.width as usize, self.height as usize);
        let need = self.layout.len(w, h);
        if self.data.len() < need {
            return Err(Error::BadFrame(format!(
                "{:?} {w}x{h}: {} bytes, {need} needed",
                self.layout,
                self.data.len()
            )));
        }
        let (cw, ch) = (w.div_ceil(2), h.div_ceil(2));
        Ok(match self.layout {
            // I420's chroma stride is half the luma's, so an odd width cannot
            // go out as it is: pad the luma rows by one.
            Layout::Yuv420p if w % 2 == 0 => (FourCC::I420, w, self.data[..need].to_vec()),
            Layout::Yuv420p => {
                let stride = w + 1;
                let mut out = Vec::with_capacity(stride * h + 2 * cw * ch);
                for r in 0..h {
                    out.extend_from_slice(&self.data[r * w..r * w + w]);
                    out.push(self.data[r * w + w - 1]);
                }
                out.extend_from_slice(&self.data[w * h..need]);
                (FourCC::I420, stride, out)
            }
            Layout::Nv12 if w % 2 == 0 => (FourCC::Nv12, w, self.data[..need].to_vec()),
            Layout::Nv12 => {
                // Rows of `2 * cw` bytes for both planes.
                let stride = 2 * cw;
                let mut out = Vec::with_capacity(stride * (h + ch));
                for r in 0..h {
                    out.extend_from_slice(&self.data[r * w..r * w + w]);
                    out.push(self.data[r * w + w - 1]);
                }
                out.extend_from_slice(&self.data[w * h..need]);
                (FourCC::Nv12, stride, out)
            }
            Layout::Yuv422p => {
                let (y, rest) = self.data.split_at(w * h);
                let (u, v) = rest.split_at(cw * h);
                let stride = 4 * cw;
                let mut out = Vec::with_capacity(stride * h);
                for r in 0..h {
                    for i in 0..cw {
                        let y0 = y[r * w + 2 * i];
                        let y1 = y[r * w + (2 * i + 1).min(w - 1)];
                        out.extend_from_slice(&[u[r * cw + i], y0, v[r * cw + i], y1]);
                    }
                }
                (FourCC::Uyvy, stride, out)
            }
            Layout::Yuv420p10le | Layout::Yuv422p10le => {
                let chroma_rows = if self.layout == Layout::Yuv420p10le {
                    ch
                } else {
                    h
                };
                let sample = |i: usize| {
                    let v = u16::from_le_bytes([self.data[2 * i], self.data[2 * i + 1]]);
                    (v.min(1023) << 6).to_le_bytes()
                };
                // Rows of the full width rounded up to even, so the UV plane
                // (`cw` pairs of two samples) fits the same stride.
                let stride = 4 * cw;
                let mut out = vec![0u8; 2 * stride * h];
                for r in 0..h {
                    let row = &mut out[r * stride..(r + 1) * stride];
                    for x in 0..2 * cw {
                        row[2 * x..2 * x + 2].copy_from_slice(&sample(r * w + x.min(w - 1)));
                    }
                }
                let (u0, v0) = (w * h, w * h + cw * chroma_rows);
                for r in 0..h {
                    let src = if chroma_rows == h { r } else { r / 2 };
                    let row = &mut out[(h + r) * stride..(h + r + 1) * stride];
                    for i in 0..cw {
                        row[4 * i..4 * i + 2].copy_from_slice(&sample(u0 + src * cw + i));
                        row[4 * i + 2..4 * i + 4].copy_from_slice(&sample(v0 + src * cw + i));
                    }
                }
                (FourCC::P216, stride, out)
            }
            Layout::Rgba => (FourCC::Rgba, 4 * w, self.data[..need].to_vec()),
        })
    }
}

/// The colour a frame's metadata declares: NDI 6 senders of HDR (and some
/// SDR) video attach `<ndi_color_info transfer="…" matrix="…"
/// primaries="…"/>` to the frame. The values are as written (`bt_709`,
/// `bt_2020`, `bt_2100_hlg`, `bt_2100_pq`, `bt_601`, …).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ColorInfo {
    pub transfer: Option<String>,
    pub matrix: Option<String>,
    pub primaries: Option<String>,
}

impl ColorInfo {
    /// The `ndi_color_info` element in `metadata`, if any.
    pub fn parse(metadata: &str) -> Option<Self> {
        let start = metadata.find("<ndi_color_info")?;
        let rest = &metadata[start + "<ndi_color_info".len()..];
        let element = &rest[..rest.find('>')?];
        let attr = |name: &str| -> Option<String> {
            let mut at = 0;
            while let Some(i) = element[at..].find(name) {
                let i = at + i;
                let before_ok = i == 0 || element.as_bytes()[i - 1].is_ascii_whitespace();
                let after = element[i + name.len()..].trim_start();
                if before_ok && let Some(after) = after.strip_prefix('=') {
                    let after = after.trim_start();
                    let quote = after.chars().next()?;
                    if quote == '"' || quote == '\'' {
                        let body = &after[1..];
                        return Some(body[..body.find(quote)?].to_string());
                    }
                }
                at = i + name.len();
            }
            None
        };
        Some(Self {
            transfer: attr("transfer"),
            matrix: attr("matrix"),
            primaries: attr("primaries"),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uyvy_with_padded_rows_unpacks_to_planar_422() {
        // 4x2, stride 12 (8 bytes of pixels, 4 of padding).
        let mut data = Vec::new();
        for r in 0..2u8 {
            data.extend_from_slice(&[
                10 + r,
                20 + r,
                30 + r,
                21 + r,
                11 + r,
                22 + r,
                31 + r,
                23 + r,
            ]);
            data.extend_from_slice(&[0xEE; 4]);
        }
        let p = to_picture(FourCC::Uyvy, 4, 2, 12, &data).unwrap();
        assert_eq!(p.layout, Layout::Yuv422p);
        assert_eq!(
            p.data,
            [
                20, 21, 22, 23, 21, 22, 23, 24, // Y
                10, 11, 11, 12, // U
                30, 31, 31, 32, // V
            ]
        );
    }

    #[test]
    fn p216_rounds_sixteen_bits_to_ten() {
        // 2x1: Y plane then UV plane, stride 4.
        let s = |v: u16| v.to_le_bytes();
        let mut data = Vec::new();
        data.extend_from_slice(&s(0xFFFF));
        data.extend_from_slice(&s(64 << 6));
        data.extend_from_slice(&s(512 << 6));
        data.extend_from_slice(&s((448 << 6) + 31));
        let p = to_picture(FourCC::P216, 2, 1, 4, &data).unwrap();
        assert_eq!(p.layout, Layout::Yuv422p10le);
        let got: Vec<u16> = p
            .data
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        assert_eq!(got, [1023, 64, 512, 448]);
    }

    #[test]
    fn yv12_swaps_its_chroma_planes_into_i420_order() {
        // 2x2: Y(4) V(1) U(1).
        let p = to_picture(FourCC::Yv12, 2, 2, 2, &[1, 2, 3, 4, 9, 7]).unwrap();
        assert_eq!(p.data, [1, 2, 3, 4, 7, 9]);
        let p = to_picture(FourCC::I420, 2, 2, 2, &[1, 2, 3, 4, 9, 7]).unwrap();
        assert_eq!(p.data, [1, 2, 3, 4, 9, 7]);
    }

    #[test]
    fn bgrx_comes_out_as_opaque_rgba() {
        let p = to_picture(FourCC::Bgrx, 1, 1, 4, &[1, 2, 3, 0]).unwrap();
        assert_eq!(p.data, [3, 2, 1, 255]);
        let p = to_picture(FourCC::Bgra, 1, 1, 4, &[1, 2, 3, 77]).unwrap();
        assert_eq!(p.data, [3, 2, 1, 77]);
    }

    #[test]
    fn a_short_buffer_or_stride_is_an_error_not_a_panic() {
        assert!(matches!(
            to_picture(FourCC::Uyvy, 4, 2, 8, &[0; 15]),
            Err(Error::BadFrame(_))
        ));
        assert!(matches!(
            to_picture(FourCC::Uyvy, 4, 2, 6, &[0; 64]),
            Err(Error::BadFrame(_))
        ));
        assert!(matches!(
            to_picture(FourCC::Other(1), 4, 2, 8, &[0; 64]),
            Err(Error::UnsupportedFourCC(_))
        ));
    }

    #[test]
    fn yuv422p_round_trips_through_uyvy() {
        let p = Picture {
            layout: Layout::Yuv422p,
            width: 4,
            height: 2,
            data: (0..16).collect(),
        };
        let (fourcc, stride, bytes) = p.to_ndi().unwrap();
        assert_eq!((fourcc, stride), (FourCC::Uyvy, 8));
        assert_eq!(to_picture(fourcc, 4, 2, stride, &bytes).unwrap(), p);
    }

    #[test]
    fn ten_bit_420_goes_out_as_p216_and_comes_back_as_422() {
        let (w, h) = (4usize, 2usize);
        let samples: Vec<u16> = (0..(w * h + 2 * 2)).map(|i| 64 + 10 * i as u16).collect();
        let p = Picture {
            layout: Layout::Yuv420p10le,
            width: w as u32,
            height: h as u32,
            data: samples.iter().flat_map(|s| s.to_le_bytes()).collect(),
        };
        let (fourcc, stride, bytes) = p.to_ndi().unwrap();
        assert_eq!(fourcc, FourCC::P216);
        let back = to_picture(fourcc, 4, 2, stride, &bytes).unwrap();
        assert_eq!(back.layout, Layout::Yuv422p10le);
        let got: Vec<u16> = back
            .data
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        // Luma exact; each 4:2:0 chroma row repeated for both picture rows.
        assert_eq!(&got[..8], &samples[..8]);
        let (u, v) = (samples[8], samples[10]);
        let (u1, v1) = (samples[9], samples[11]);
        assert_eq!(&got[8..12], &[u, u1, u, u1]);
        assert_eq!(&got[12..16], &[v, v1, v, v1]);
    }

    #[test]
    fn odd_width_i420_is_padded_to_an_even_stride() {
        let p = Picture {
            layout: Layout::Yuv420p,
            width: 3,
            height: 2,
            data: vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10],
        };
        let (fourcc, stride, bytes) = p.to_ndi().unwrap();
        assert_eq!((fourcc, stride), (FourCC::I420, 4));
        assert_eq!(bytes, [1, 2, 3, 3, 4, 5, 6, 6, 7, 8, 9, 10]);
        let back = to_picture(fourcc, 3, 2, stride, &bytes).unwrap();
        assert_eq!(back, p);
    }

    #[test]
    fn colour_info_reads_its_attributes() {
        let xml =
            r#"<ndi_color_info transfer="bt_2100_pq" matrix='bt_2020' primaries = "bt_2020"/>"#;
        let c = ColorInfo::parse(xml).unwrap();
        assert_eq!(c.transfer.as_deref(), Some("bt_2100_pq"));
        assert_eq!(c.matrix.as_deref(), Some("bt_2020"));
        assert_eq!(c.primaries.as_deref(), Some("bt_2020"));
        assert!(ColorInfo::parse("<ndi_tally on_program=\"true\"/>").is_none());
    }

    #[test]
    fn fourcc_round_trips_and_prints() {
        for f in [FourCC::Uyvy, FourCC::P216, FourCC::I420, FourCC::Bgrx] {
            assert_eq!(FourCC::from_raw(f.raw()), f);
        }
        assert_eq!(FourCC::Uyva.to_string(), "UYVA");
        assert_eq!(FourCC::Other(1).to_string(), "0x00000001");
    }
}
