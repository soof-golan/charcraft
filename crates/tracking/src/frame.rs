//! Camera frames.

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

/// Largest frame accepted, in pixels (an 8K frame), so hostile sizes cannot exhaust memory.
pub const MAX_FRAME_PIXELS: usize = 8192 * 4608;

/// An RGB frame, 8 bits per channel, row-major, no padding.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Frame {
    width: u32,
    height: u32,
    rgb: Vec<u8>,
}

impl Frame {
    /// A frame from packed RGB bytes.
    ///
    /// # Errors
    /// [`Error::BadFrame`] when the buffer length is not `width * height * 3`, [`Error::FrameTooLarge`] above
    /// [`MAX_FRAME_PIXELS`].
    pub fn new(width: u32, height: u32, rgb: Vec<u8>) -> Result<Self> {
        let pixels = pixel_count(width, height);
        if pixels.is_none_or(|p| p > MAX_FRAME_PIXELS) {
            return Err(Error::FrameTooLarge { width, height, max: MAX_FRAME_PIXELS });
        }
        let expected = pixels.and_then(|p| p.checked_mul(3)).unwrap_or(usize::MAX);
        if rgb.len() != expected {
            return Err(Error::BadFrame { width, height, expected, got: rgb.len() });
        }
        Ok(Self { width, height, rgb })
    }

    /// A frame filled with one colour (tests, placeholders).
    ///
    /// # Errors
    /// As [`Frame::new`].
    pub fn solid(width: u32, height: u32, colour: [u8; 3]) -> Result<Self> {
        let pixels = pixel_count(width, height).filter(|p| *p <= MAX_FRAME_PIXELS).ok_or(Error::FrameTooLarge { width, height, max: MAX_FRAME_PIXELS })?;
        Self::new(width, height, colour.iter().copied().cycle().take(pixels.saturating_mul(3)).collect())
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

    /// The packed RGB bytes.
    #[must_use]
    pub fn rgb(&self) -> &[u8] {
        &self.rgb
    }

    /// A copy no wider than `max_width` pixels (nearest-neighbour; the same frame when it already fits). For the
    /// camera preview, which only has to be recognisable.
    #[must_use]
    pub fn downscaled(&self, max_width: u32) -> Self {
        let max_width = max_width.max(1);
        if self.width <= max_width || self.width == 0 || self.height == 0 {
            return self.clone();
        }
        let out_w = max_width;
        let scaled = |value: u32, num: u32, den: u32| {
            u32::try_from(u64::from(value).saturating_mul(u64::from(num)).checked_div(u64::from(den)).unwrap_or(0)).unwrap_or(0)
        };
        let out_h = scaled(self.height, out_w, self.width).max(1);
        let mut rgb = Vec::with_capacity(usize::try_from(out_w).unwrap_or(0).saturating_mul(usize::try_from(out_h).unwrap_or(0)).saturating_mul(3));
        for y in 0..out_h {
            let sy = scaled(y, self.height, out_h);
            for x in 0..out_w {
                let sx = scaled(x, self.width, out_w);
                rgb.extend_from_slice(&self.pixel(sx, sy).unwrap_or([0; 3]));
            }
        }
        Self { width: out_w, height: out_h, rgb }
    }

    /// The pixel at `(x, y)`, or `None` outside the frame.
    #[must_use]
    pub fn pixel(&self, x: u32, y: u32) -> Option<[u8; 3]> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let index = usize::try_from(y).ok()?.checked_mul(usize::try_from(self.width).ok()?)?.checked_add(usize::try_from(x).ok()?)?;
        let start = index.checked_mul(3)?;
        let px = self.rgb.get(start..start.checked_add(3)?)?;
        Some([*px.first()?, *px.get(1)?, *px.get(2)?])
    }
}

fn pixel_count(width: u32, height: u32) -> Option<usize> {
    usize::try_from(width).ok()?.checked_mul(usize::try_from(height).ok()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checks_the_buffer_length() {
        assert!(Frame::new(2, 2, vec![0; 12]).is_ok());
        assert!(matches!(Frame::new(2, 2, vec![0; 11]), Err(Error::BadFrame { .. })));
        assert!(matches!(Frame::new(u32::MAX, u32::MAX, vec![]), Err(Error::FrameTooLarge { .. })));
        assert!(matches!(Frame::solid(u32::MAX, 2, [0; 3]), Err(Error::FrameTooLarge { .. })));
    }

    #[test]
    fn downscaling_keeps_the_aspect_and_the_colours() {
        let f = Frame::solid(640, 480, [10, 20, 30]).unwrap();
        let small = f.downscaled(160);
        assert_eq!((small.width(), small.height()), (160, 120));
        assert_eq!(small.pixel(159, 119), Some([10, 20, 30]));
        assert_eq!(f.downscaled(1000), f, "a frame that fits is left alone");
        assert_eq!(f.downscaled(0).width(), 1);
    }

    #[test]
    fn reads_pixels() {
        let f = Frame::new(2, 1, vec![1, 2, 3, 4, 5, 6]).unwrap();
        assert_eq!(f.pixel(1, 0), Some([4, 5, 6]));
        assert_eq!(f.pixel(2, 0), None);
        assert_eq!(f.pixel(0, 1), None);
        assert_eq!(Frame::solid(2, 2, [9, 8, 7]).unwrap().pixel(1, 1), Some([9, 8, 7]));
    }
}
