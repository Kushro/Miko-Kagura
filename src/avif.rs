//! AVIF input decoding. The `image` crate only encodes AVIF, and some sources
//! serve their pages as AVIF. avif-parse reads the container; the AV1 payload
//! goes through rav1d, a pure-Rust port of dav1d, so packaging needs no C
//! library or assembler.
//!
//! Covers still images: 8/10/12-bit, 4:0:0/4:2:0/4:2:2/4:4:4, full or limited
//! range, the common matrix coefficients, and an optional alpha item.
//! Rotation, mirroring and cropping properties are ignored.

use std::borrow::Cow;
use std::mem::MaybeUninit;
use std::ptr::NonNull;

use image::{DynamicImage, RgbImage, RgbaImage};
use rav1d::include::dav1d::data::Dav1dData;
use rav1d::include::dav1d::dav1d::{Dav1dContext, Dav1dSettings};
use rav1d::include::dav1d::headers::{
    DAV1D_MC_BT2020_CL, DAV1D_MC_BT2020_NCL, DAV1D_MC_BT709, DAV1D_MC_FCC, DAV1D_MC_IDENTITY,
    DAV1D_MC_SMPTE240, DAV1D_PIXEL_LAYOUT_I400, DAV1D_PIXEL_LAYOUT_I420, DAV1D_PIXEL_LAYOUT_I422,
};
use rav1d::include::dav1d::picture::Dav1dPicture;
use rav1d::src::lib::{
    dav1d_close, dav1d_data_create, dav1d_data_unref, dav1d_default_settings, dav1d_get_picture,
    dav1d_open, dav1d_picture_unref, dav1d_send_data,
};

/// Whether `data` starts with an ISO-BMFF `ftyp` box naming an AVIF brand.
pub fn is_avif(data: &[u8]) -> bool {
    if data.len() < 16 || &data[4..8] != b"ftyp" {
        return false;
    }
    let size = u32::from_be_bytes([data[0], data[1], data[2], data[3]]) as usize;
    let end = size.clamp(16, data.len());
    // Major brand, then the compatible brands after the minor version.
    let major: &[u8; 4] = data[8..12].try_into().unwrap_or(&[0; 4]);
    std::iter::once(major)
        .chain(data[16..end].as_chunks::<4>().0)
        .any(|brand| brand == b"avif" || brand == b"avis")
}

/// avif-parse only accepts the `avif` major brand, but encoders also write
/// AVIF stills under a generic one (`mif1`, `miaf`) with `avif` listed as
/// compatible, and `avis` sequences carry a still primary item too.
/// [`is_avif`] has already accepted those, so present them to the parser
/// with the major brand it expects.
fn with_avif_major_brand(data: &[u8]) -> Cow<'_, [u8]> {
    if data.len() < 12 || &data[4..8] != b"ftyp" || &data[8..12] == b"avif" {
        return Cow::Borrowed(data);
    }
    let mut owned = data.to_vec();
    owned[8..12].copy_from_slice(b"avif");
    Cow::Owned(owned)
}

fn read_avif(data: &[u8]) -> avif_parse::Result<avif_parse::AvifData> {
    avif_parse::read_avif(&mut &with_avif_major_brand(data)[..])
}

/// Width and height from the AV1 sequence header, without decoding pixels.
pub fn dimensions(data: &[u8]) -> Option<(u32, u32)> {
    let avif = read_avif(data).ok()?;
    let meta = avif.primary_item_metadata().ok()?;
    Some((meta.max_frame_width.get(), meta.max_frame_height.get()))
}

/// Decode an AVIF still image to 8-bit RGB, or RGBA when it has an alpha item.
pub fn decode(data: &[u8]) -> Result<DynamicImage, String> {
    let avif = read_avif(data).map_err(|e| format!("Invalid AVIF: {e:?}"))?;
    let color = Picture::decode(&avif.primary_item)?;
    let (w, h) = color.size();
    let rgb = color.to_rgb();

    let Some(alpha_item) = avif.alpha_item.as_deref() else {
        let img = RgbImage::from_raw(w, h, rgb).ok_or("AVIF size mismatch")?;
        return Ok(DynamicImage::ImageRgb8(img));
    };
    let alpha = Picture::decode(alpha_item)?;
    if alpha.size() != (w, h) {
        return Err("AVIF alpha plane does not match the image size".to_string());
    }
    let alpha = alpha.to_gray();
    let mut rgba = Vec::with_capacity(w as usize * h as usize * 4);
    for (&px, &a) in rgb.as_chunks::<3>().0.iter().zip(&alpha) {
        let [r, g, b] = px.map(|c| {
            if avif.premultiplied_alpha && a > 0 {
                ((c as u32 * 255 + a as u32 / 2) / a as u32).min(255) as u8
            } else {
                c
            }
        });
        rgba.extend_from_slice(&[r, g, b, a]);
    }
    let img = RgbaImage::from_raw(w, h, rgba).ok_or("AVIF size mismatch")?;
    Ok(DynamicImage::ImageRgba8(img))
}

/// rav1d reports "feed me more / call again" as `-EAGAIN`.
const EAGAIN: i32 = -libc::EAGAIN;

struct Context(Option<Dav1dContext>);

impl Drop for Context {
    fn drop(&mut self) {
        // SAFETY: the context came from `dav1d_open` and is closed only here.
        unsafe { dav1d_close(Some(NonNull::from(&mut self.0))) };
    }
}

/// A decoded AV1 frame, released on drop.
struct Picture(Dav1dPicture);

impl Drop for Picture {
    fn drop(&mut self) {
        // SAFETY: the picture came from `dav1d_get_picture` and is released only here.
        unsafe { dav1d_picture_unref(Some(NonNull::from(&mut self.0))) };
    }
}

impl Picture {
    /// Decode one AV1 still frame from its OBU bitstream.
    fn decode(obu: &[u8]) -> Result<Picture, String> {
        if obu.is_empty() {
            return Err("Empty AV1 payload".to_string());
        }
        // SAFETY: the dav1d API contract — settings initialised by
        // `dav1d_default_settings`, data buffer from `dav1d_data_create` sized
        // for the copy, and every handle released by the guards above.
        unsafe {
            let mut settings = MaybeUninit::<Dav1dSettings>::uninit();
            dav1d_default_settings(NonNull::new_unchecked(settings.as_mut_ptr()));
            let mut settings = settings.assume_init();
            // One still frame: threads would only add start-up cost.
            settings.n_threads = 1;
            settings.max_frame_delay = 1;

            let mut ctx = Context(None);
            let res = dav1d_open(
                Some(NonNull::from(&mut ctx.0)),
                Some(NonNull::from(&mut settings)),
            );
            let handle = match (res.0, ctx.0) {
                (0, Some(handle)) => handle,
                _ => return Err(format!("AV1 decoder failed to start ({})", res.0)),
            };

            let mut data = Dav1dData::default();
            let buf = dav1d_data_create(Some(NonNull::from(&mut data)), obu.len());
            if buf.is_null() {
                return Err("AV1 decoder could not allocate input".to_string());
            }
            std::ptr::copy_nonoverlapping(obu.as_ptr(), buf, obu.len());

            let mut pic = Picture(Dav1dPicture::default());
            let mut outcome = Err("AV1 payload produced no picture".to_string());
            for _ in 0..64 {
                if data.sz > 0 {
                    let res = dav1d_send_data(Some(handle), Some(NonNull::from(&mut data)));
                    if res.0 < 0 && res.0 != EAGAIN {
                        outcome = Err(format!("AV1 decode failed ({})", res.0));
                        break;
                    }
                }
                let res = dav1d_get_picture(Some(handle), Some(NonNull::from(&mut pic.0)));
                if res.0 == 0 {
                    outcome = Ok(());
                    break;
                }
                if res.0 != EAGAIN {
                    outcome = Err(format!("AV1 decode failed ({})", res.0));
                    break;
                }
            }
            if data.sz > 0 {
                dav1d_data_unref(Some(NonNull::from(&mut data)));
            }
            outcome.map(|()| pic)
        }
    }

    fn size(&self) -> (u32, u32) {
        (self.0.p.w.max(0) as u32, self.0.p.h.max(0) as u32)
    }

    /// Sample `(x, y)` of `plane` (0 = Y, 1 = U, 2 = V) at its native depth.
    fn sample(&self, plane: usize, x: usize, y: usize) -> u32 {
        let stride = self.0.stride[(plane > 0) as usize];
        let Some(base) = self.0.data[plane] else {
            return 0;
        };
        // SAFETY: (x, y) is within the plane, whose rows are `stride` bytes apart.
        unsafe {
            let row = base.as_ptr().cast::<u8>().offset(y as isize * stride);
            if self.0.p.bpc > 8 {
                row.cast::<u16>().add(x).read_unaligned() as u32
            } else {
                *row.add(x) as u32
            }
        }
    }

    /// `(matrix coefficients, full range)` from the sequence header.
    fn color_info(&self) -> (u32, bool) {
        match self.0.seq_hdr {
            // SAFETY: the header lives as long as the picture holds its reference.
            Some(hdr) => unsafe {
                let hdr = hdr.as_ref();
                (hdr.mtrx, hdr.color_range != 0)
            },
            None => (DAV1D_MC_BT709, false),
        }
    }

    /// Normalised luma at `(x, y)`, range-expanded to 0.0–1.0.
    fn luma(&self, x: usize, y: usize, full: bool, max: f32) -> f32 {
        let v = self.sample(0, x, y) as f32;
        let unit = max / 255.0;
        if full {
            v / max
        } else {
            (v - 16.0 * unit) / (219.0 * unit)
        }
    }

    /// The luma plane as 8-bit gray (alpha items are monochrome AV1).
    fn to_gray(&self) -> Vec<u8> {
        let (w, h) = self.size();
        let (_, full) = self.color_info();
        let max = ((1u32 << self.0.p.bpc) - 1) as f32;
        let mut out = Vec::with_capacity(w as usize * h as usize);
        for y in 0..h as usize {
            for x in 0..w as usize {
                out.push(to_u8(self.luma(x, y, full, max)));
            }
        }
        out
    }

    /// Convert to packed 8-bit RGB.
    fn to_rgb(&self) -> Vec<u8> {
        let (w, h) = self.size();
        let (mtrx, full) = self.color_info();
        let max = ((1u32 << self.0.p.bpc) - 1) as f32;
        let unit = max / 255.0;
        let layout = self.0.p.layout;
        let (sx, sy) = match layout {
            DAV1D_PIXEL_LAYOUT_I420 => (1, 1),
            DAV1D_PIXEL_LAYOUT_I422 => (1, 0),
            _ => (0, 0),
        };
        // (Kr, Kb) for Y'CbCr → R'G'B'. Unspecified matrices are treated as
        // BT.601, as libavif and browsers do.
        let (kr, kb) = match mtrx {
            DAV1D_MC_BT709 => (0.2126, 0.0722),
            DAV1D_MC_FCC => (0.30, 0.11),
            DAV1D_MC_SMPTE240 => (0.212, 0.087),
            DAV1D_MC_BT2020_NCL | DAV1D_MC_BT2020_CL => (0.2627, 0.0593),
            _ => (0.299, 0.114),
        };
        let kg = 1.0 - kr - kb;

        let mut out = Vec::with_capacity(w as usize * h as usize * 3);
        for y in 0..h as usize {
            for x in 0..w as usize {
                let luma = self.luma(x, y, full, max);
                if layout == DAV1D_PIXEL_LAYOUT_I400 {
                    let v = to_u8(luma);
                    out.extend_from_slice(&[v, v, v]);
                    continue;
                }
                let chroma = |plane| {
                    let v = self.sample(plane, x >> sx, y >> sy) as f32;
                    if full {
                        (v - (max + 1.0) / 2.0) / max
                    } else {
                        (v - 128.0 * unit) / (224.0 * unit)
                    }
                };
                let (cb, cr) = (chroma(1), chroma(2));
                let (r, g, b) = if mtrx == DAV1D_MC_IDENTITY {
                    // GBR stored as Y=G, U=B, V=R.
                    let offset = 0.5;
                    (cr + offset, luma, cb + offset)
                } else {
                    let r = luma + 2.0 * (1.0 - kr) * cr;
                    let b = luma + 2.0 * (1.0 - kb) * cb;
                    let g = (luma - kr * r - kb * b) / kg;
                    (r, g, b)
                };
                out.extend_from_slice(&[to_u8(r), to_u8(g), to_u8(b)]);
            }
        }
        out
    }
}

fn to_u8(v: f32) -> u8 {
    (v * 255.0 + 0.5).clamp(0.0, 255.0) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognises_avif_brands() {
        let mut ftyp = Vec::new();
        ftyp.extend_from_slice(&24u32.to_be_bytes());
        ftyp.extend_from_slice(b"ftypmif1\0\0\0\0mif1avif");
        assert!(is_avif(&ftyp));
        assert!(!is_avif(b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR"));
        let mut heic = ftyp.clone();
        heic[20..24].copy_from_slice(b"heic");
        assert!(!is_avif(&heic));
    }

    /// Round-trips an image through the `image` crate's AVIF encoder.
    #[test]
    fn decodes_what_the_image_encoder_writes() {
        let src = RgbImage::from_fn(64, 48, |x, y| {
            image::Rgb([(x * 4) as u8, (y * 5) as u8, 128])
        });
        let mut buf = std::io::Cursor::new(Vec::new());
        DynamicImage::ImageRgb8(src.clone())
            .write_to(&mut buf, image::ImageFormat::Avif)
            .unwrap();
        let bytes = buf.into_inner();
        assert!(is_avif(&bytes));
        assert_eq!(dimensions(&bytes), Some((64, 48)));

        let decoded = decode(&bytes).unwrap().to_rgb8();
        assert_eq!(decoded.dimensions(), (64, 48));
        let err: f64 = src
            .pixels()
            .zip(decoded.pixels())
            .flat_map(|(a, b)| (0..3).map(move |c| (a[c] as f64 - b[c] as f64).abs()))
            .sum::<f64>()
            / (64.0 * 48.0 * 3.0);
        assert!(err < 6.0, "mean abs error {err}");
    }

    /// AVIF stills under a generic major brand, with `avif` only compatible.
    #[test]
    fn decodes_a_generic_major_brand() {
        let src = RgbImage::from_fn(16, 16, |x, y| {
            image::Rgb([(x * 16) as u8, (y * 16) as u8, 64])
        });
        let mut buf = std::io::Cursor::new(Vec::new());
        DynamicImage::ImageRgb8(src)
            .write_to(&mut buf, image::ImageFormat::Avif)
            .unwrap();
        let mut bytes = buf.into_inner();
        // The encoder writes `avif` as major and `mif1 miaf` as compatible
        // brands; list `avif` among the compatible ones instead.
        assert_eq!(&bytes[4..24], b"ftypavif\0\0\0\0mif1miaf");
        bytes[16..20].copy_from_slice(b"avif");
        for major in [b"mif1", b"miaf"] {
            bytes[8..12].copy_from_slice(major);
            assert!(is_avif(&bytes));
            assert_eq!(dimensions(&bytes), Some((16, 16)));
            assert_eq!(decode(&bytes).unwrap().width(), 16);
        }
    }

    #[test]
    fn keeps_alpha() {
        let src = RgbaImage::from_fn(32, 32, |x, _| image::Rgba([200, 100, 50, (x * 8) as u8]));
        let mut buf = std::io::Cursor::new(Vec::new());
        DynamicImage::ImageRgba8(src)
            .write_to(&mut buf, image::ImageFormat::Avif)
            .unwrap();
        let decoded = decode(&buf.into_inner()).unwrap();
        assert!(decoded.color().has_alpha());
        let px = decoded.to_rgba8();
        assert!(px.get_pixel(0, 0)[3] < 16);
        assert!(px.get_pixel(31, 0)[3] > 230);
    }

    /// Compares against reference decodes: every `name.avif` in
    /// `MIKO_AVIF_SAMPLES` next to a `name.ref.png` (e.g. from libavif's `avifdec`).
    /// `MIKO_AVIF_SAMPLES=… cargo test matches_reference_decodes -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn matches_reference_decodes() {
        let dir = std::env::var("MIKO_AVIF_SAMPLES").expect("MIKO_AVIF_SAMPLES");
        let mut checked = 0;
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().is_none_or(|e| e != "avif") {
                continue;
            }
            let reference = image::open(path.with_extension("ref.png")).unwrap().to_rgba8();
            let start = std::time::Instant::now();
            let ours = decode(&std::fs::read(&path).unwrap()).unwrap().to_rgba8();
            let elapsed = start.elapsed();
            assert_eq!(ours.dimensions(), reference.dimensions(), "{}", path.display());
            let (mut sum, mut worst) = (0u64, 0u8);
            for (i, (a, b)) in ours.as_raw().iter().zip(reference.as_raw()).enumerate() {
                // Colour under (nearly) transparent pixels is noise once
                // un-premultiplied, and invisible either way.
                if i % 4 != 3 && reference.as_raw()[i | 3] < 16 {
                    continue;
                }
                let d = a.abs_diff(*b);
                sum += d as u64;
                worst = worst.max(d);
            }
            let mean = sum as f64 / ours.as_raw().len() as f64;
            println!("{}: mean {mean:.2}, max {worst}, {elapsed:.2?}", path.display());
            // Edges differ by more than the mean: libavif upsamples chroma
            // bilinearly, this decoder takes the nearest sample.
            assert!(mean < 1.5 && worst < 96, "{} differs from its reference", path.display());
            checked += 1;
        }
        assert!(checked > 0, "no samples found");
    }

    #[test]
    fn rejects_garbage() {
        assert!(decode(b"not an image").is_err());
    }
}
