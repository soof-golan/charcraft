//! Pictures on their way to an output: straight-alpha RGBA, fitted to the output canvas, flattened or converted to
//! the pixel format a sink wants.

use image::imageops::{self, FilterType};
use image::{ImageEncoder, RgbaImage};

use crate::Error;

/// Largest side of a picture, in pixels.
pub const MAX_SIDE: u32 = 8192;

/// The colour put behind a transparent picture for outputs that cannot carry alpha (a chroma key green).
pub const KEY_GREEN: [u8; 3] = [0x00, 0xb1, 0x40];

/// An RGBA picture, 8 bits per channel, straight (not premultiplied) alpha, rows top to bottom.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Picture {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}

impl Picture {
    /// A picture from packed RGBA bytes.
    ///
    /// # Errors
    /// [`Error::BadPicture`] when a side is zero or above [`MAX_SIDE`], or the buffer is not `width * height * 4`.
    pub fn new(width: u32, height: u32, rgba: Vec<u8>) -> Result<Self, Error> {
        let needed = usize::try_from(width).ok().and_then(|w| w.checked_mul(usize::try_from(height).ok()?)).and_then(|p| p.checked_mul(4));
        if width == 0 || height == 0 || width > MAX_SIDE || height > MAX_SIDE || needed != Some(rgba.len()) {
            return Err(Error::BadPicture { width, height, got: rgba.len() });
        }
        Ok(Self { width, height, rgba })
    }

    /// A picture filled with one RGBA colour.
    ///
    /// # Errors
    /// As [`Picture::new`].
    pub fn solid(width: u32, height: u32, colour: [u8; 4]) -> Result<Self, Error> {
        let pixels = usize::try_from(width).ok().and_then(|w| w.checked_mul(usize::try_from(height).ok()?)).filter(|p| *p <= 8192 * 8192);
        let rgba = colour.iter().copied().cycle().take(pixels.unwrap_or(0).saturating_mul(4)).collect();
        Self::new(width, height, rgba)
    }

    /// Width in pixels.
    #[must_use]
    pub const fn width(&self) -> u32 {
        self.width
    }

    /// Height in pixels.
    #[must_use]
    pub const fn height(&self) -> u32 {
        self.height
    }

    /// The packed RGBA bytes.
    #[must_use]
    pub fn rgba(&self) -> &[u8] {
        &self.rgba
    }

    /// The pixel at `(x, y)`.
    #[must_use]
    pub fn pixel(&self, x: u32, y: u32) -> Option<[u8; 4]> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let index = usize::try_from(y).ok()?.checked_mul(usize::try_from(self.width).ok()?)?.checked_add(usize::try_from(x).ok()?)?.checked_mul(4)?;
        let px = self.rgba.get(index..index.checked_add(4)?)?;
        Some([*px.first()?, *px.get(1)?, *px.get(2)?, *px.get(3)?])
    }

    /// This picture scaled to fit inside a `width` by `height` canvas, centred, with transparent bars where the
    /// aspect ratios differ.
    ///
    /// # Errors
    /// [`Error::BadPicture`] for a canvas side that is zero or above [`MAX_SIDE`].
    pub fn fitted(&self, width: u32, height: u32) -> Result<Self, Error> {
        if width == 0 || height == 0 || width > MAX_SIDE || height > MAX_SIDE {
            return Err(Error::BadPicture { width, height, got: 0 });
        }
        if (self.width, self.height) == (width, height) {
            return Ok(self.clone());
        }
        let scale = (f64::from(width) / f64::from(self.width)).min(f64::from(height) / f64::from(self.height));
        let fit_w = anim_core::num::round_f64_to_u32(f64::from(self.width) * scale).clamp(1, width);
        let fit_h = anim_core::num::round_f64_to_u32(f64::from(self.height) * scale).clamp(1, height);
        let Some(source) = RgbaImage::from_raw(self.width, self.height, self.rgba.clone()) else {
            return Err(Error::BadPicture { width: self.width, height: self.height, got: self.rgba.len() });
        };
        let scaled = imageops::resize(&source, fit_w, fit_h, FilterType::Triangle);
        let mut canvas = RgbaImage::new(width, height);
        imageops::replace(&mut canvas, &scaled, i64::from(width.saturating_sub(fit_w) / 2), i64::from(height.saturating_sub(fit_h) / 2));
        Self::new(width, height, canvas.into_raw())
    }

    /// Packed RGB with the picture composited over `backdrop`.
    #[must_use]
    pub fn flattened(&self, backdrop: [u8; 3]) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.rgba.len().saturating_div(4).saturating_mul(3));
        for px in self.rgba.as_chunks::<4>().0 {
            let a = u16::from(px.get(3).copied().unwrap_or(255));
            for (channel, back) in px.iter().take(3).zip(backdrop) {
                let mixed = (u16::from(*channel).saturating_mul(a)).saturating_add(u16::from(back).saturating_mul(255_u16.saturating_sub(a))) / 255;
                out.push(u8::try_from(mixed.min(255)).unwrap_or(255));
            }
        }
        out
    }

    /// The picture as packed YUYV (YUV 4:2:2, BT.601 limited range) over `backdrop`, which is what webcams deliver and
    /// what virtual cameras take. The width must be even.
    ///
    /// # Errors
    /// [`Error::BadPicture`] for an odd width.
    pub fn to_yuyv(&self, backdrop: [u8; 3]) -> Result<Vec<u8>, Error> {
        if !self.width.is_multiple_of(2) {
            return Err(Error::BadPicture { width: self.width, height: self.height, got: self.rgba.len() });
        }
        let rgb = self.flattened(backdrop);
        let luma = |r: f32, g: f32, b: f32| 16.0 + 0.257 * r + 0.504 * g + 0.098 * b;
        let chroma_u = |r: f32, g: f32, b: f32| 128.0 - 0.148 * r - 0.291 * g + 0.439 * b;
        let chroma_v = |r: f32, g: f32, b: f32| 128.0 + 0.439 * r - 0.368 * g - 0.071 * b;
        let byte = |v: f32| u8::try_from(anim_core::num::round_f64_to_u32(f64::from(v).clamp(0.0, 255.0))).unwrap_or(0);
        let mut out = Vec::with_capacity(rgb.len().saturating_div(3).saturating_mul(2));
        for pair in rgb.as_chunks::<6>().0 {
            let v = |i: usize| pair.get(i).map_or(0.0, |b| f32::from(*b));
            let (r0, g0, b0, r1, g1, b1) = (v(0), v(1), v(2), v(3), v(4), v(5));
            let (r, g, b) = (f32::midpoint(r0, r1), f32::midpoint(g0, g1), f32::midpoint(b0, b1));
            out.extend_from_slice(&[byte(luma(r0, g0, b0)), byte(chroma_u(r, g, b)), byte(luma(r1, g1, b1)), byte(chroma_v(r, g, b))]);
        }
        Ok(out)
    }

    /// The picture as a PNG with its alpha.
    ///
    /// # Errors
    /// [`Error::Encode`] when encoding fails.
    pub fn to_png(&self) -> Result<Vec<u8>, Error> {
        let mut out = Vec::new();
        image::codecs::png::PngEncoder::new(&mut out)
            .write_image(&self.rgba, self.width, self.height, image::ExtendedColorType::Rgba8)
            .map_err(|e| Error::Encode(e.to_string()))?;
        Ok(out)
    }

    /// The picture as a JPEG over `backdrop`, at `quality` 1 to 100.
    ///
    /// # Errors
    /// [`Error::Encode`] when encoding fails.
    pub fn to_jpeg(&self, backdrop: [u8; 3], quality: u8) -> Result<Vec<u8>, Error> {
        let mut out = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, quality.clamp(1, 100))
            .encode(&self.flattened(backdrop), self.width, self.height, image::ExtendedColorType::Rgb8)
            .map_err(|e| Error::Encode(e.to_string()))?;
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_picture_checks_its_buffer_and_size() {
        assert!(Picture::new(2, 2, vec![0; 16]).is_ok());
        assert!(Picture::new(2, 2, vec![0; 15]).is_err());
        assert!(Picture::new(0, 2, vec![]).is_err());
        assert!(Picture::new(MAX_SIDE + 1, 1, vec![]).is_err());
        assert_eq!(Picture::solid(2, 1, [1, 2, 3, 4]).unwrap().pixel(1, 0), Some([1, 2, 3, 4]));
        assert_eq!(Picture::solid(2, 1, [1, 2, 3, 4]).unwrap().pixel(2, 0), None);
    }

    #[test]
    fn fitting_letterboxes_with_transparent_bars() {
        let wide = Picture::solid(200, 100, [255, 0, 0, 255]).unwrap();
        let out = wide.fitted(100, 100).unwrap();
        assert_eq!((out.width(), out.height()), (100, 100));
        assert_eq!(out.pixel(50, 50), Some([255, 0, 0, 255]), "the picture fills the middle");
        assert_eq!(out.pixel(50, 2).map(|p| p[3]), Some(0), "the bar above is transparent");
        assert_eq!(wide.fitted(200, 100).unwrap(), wide, "a picture of the canvas size is unchanged");
        assert!(wide.fitted(0, 10).is_err());
    }

    #[test]
    fn flattening_mixes_by_alpha() {
        let p = Picture::new(2, 1, vec![255, 0, 0, 255, 255, 0, 0, 0]).unwrap();
        assert_eq!(p.flattened([0, 0, 255]), vec![255, 0, 0, 0, 0, 255]);
        let half = Picture::new(1, 1, vec![200, 200, 200, 128]).unwrap().flattened([0, 0, 0]);
        assert!(half.iter().all(|c| (99..=101).contains(c)), "{half:?}");
    }

    #[test]
    fn yuyv_has_two_bytes_per_pixel_and_the_right_greys() {
        let white = Picture::solid(4, 2, [255, 255, 255, 255]).unwrap().to_yuyv(KEY_GREEN).unwrap();
        assert_eq!(white.len(), 4 * 2 * 2);
        assert_eq!(&white[..4], &[235, 128, 235, 128]);
        let black = Picture::solid(2, 1, [0, 0, 0, 255]).unwrap().to_yuyv(KEY_GREEN).unwrap();
        assert_eq!(&black[..4], &[16, 128, 16, 128]);
        assert!(Picture::solid(3, 1, [0; 4]).unwrap().to_yuyv(KEY_GREEN).is_err(), "odd widths cannot be 4:2:2");
        let key = Picture::solid(2, 1, [0, 0, 0, 0]).unwrap().to_yuyv(KEY_GREEN).unwrap();
        assert!(key[1] < 128 && key[3] < 128, "a transparent picture shows the green key: U and V are below grey for green");
    }

    #[test]
    fn png_keeps_alpha_and_jpeg_flattens() {
        let p = Picture::new(2, 1, vec![10, 20, 30, 0, 200, 100, 50, 255]).unwrap();
        let png = image::load_from_memory_with_format(&p.to_png().unwrap(), image::ImageFormat::Png).unwrap().to_rgba8();
        assert_eq!(png.get_pixel(0, 0).0[3], 0);
        assert_eq!(png.get_pixel(1, 0).0, [200, 100, 50, 255]);
        let jpeg = p.to_jpeg(KEY_GREEN, 90).unwrap();
        assert_eq!(&jpeg[..2], &[0xff, 0xd8], "a JPEG starts with the SOI marker");
        let back = image::load_from_memory_with_format(&jpeg, image::ImageFormat::Jpeg).unwrap().to_rgb8();
        assert!(back.get_pixel(0, 0).0[1] > 120, "the transparent pixel became the green key");
    }
}
