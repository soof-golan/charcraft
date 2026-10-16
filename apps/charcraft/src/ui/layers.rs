//! Layer thumbnails for the rig tree.
//!
//! A thumbnail is the part of the puppet's current picture that lies inside the layer's bounds, so it needs no
//! pictures of its own and always matches what the stage shows.

use anim_core::num::f64_to_f32;
use charcraft_puppet::Layer;
use egui::{Color32, Rect, Sense, TextureHandle, Ui, pos2, vec2};

/// Edge of a thumbnail box, in points.
const BOX: f32 = 22.0;

/// The part of a `size`-pixel picture that `bounds` covers, as texture coordinates, clamped to the picture.
#[must_use]
pub fn uv_of(bounds: anim_core::Rect, size: anim_core::Size) -> Option<Rect> {
    if size.width <= 0.0 || size.height <= 0.0 || bounds.x1 <= bounds.x0 || bounds.y1 <= bounds.y0 {
        return None;
    }
    let u = |v: f64, extent: f64| f64_to_f32((v / extent).clamp(0.0, 1.0));
    let uv = Rect::from_min_max(pos2(u(bounds.x0, size.width), u(bounds.y0, size.height)), pos2(u(bounds.x1, size.width), u(bounds.y1, size.height)));
    (uv.width() > 0.0 && uv.height() > 0.0).then_some(uv)
}

/// Draws the thumbnail of a layer, or an empty box when it has no bounds or there is no picture yet.
pub fn thumbnail(ui: &mut Ui, layer: &Layer, size: anim_core::Size, texture: Option<&TextureHandle>) {
    let (rect, _) = ui.allocate_exact_size(vec2(BOX, BOX), Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 3.0, Color32::from_gray(0x80).gamma_multiply(0.25));
    let (Some(texture), Some(bounds)) = (texture, layer.bounds) else { return };
    let Some(uv) = uv_of(bounds, size) else { return };
    let aspect = (f64_to_f32(bounds.x1 - bounds.x0) / f64_to_f32(bounds.y1 - bounds.y0).max(f32::EPSILON)).max(0.01);
    let fitted = if aspect >= 1.0 { vec2(BOX - 2.0, (BOX - 2.0) / aspect) } else { vec2((BOX - 2.0) * aspect, BOX - 2.0) };
    painter.image(texture.id(), Rect::from_center_size(rect.center(), fitted), uv, Color32::WHITE);
}

#[cfg(test)]
mod tests {
    use anim_core::{Rect as ArtRect, Size};

    use super::*;

    #[test]
    fn bounds_map_to_texture_coordinates_inside_the_picture() {
        let uv = uv_of(ArtRect { x0: 50.0, y0: 25.0, x1: 100.0, y1: 50.0 }, Size::new(100.0, 100.0)).unwrap();
        assert_eq!((uv.min.x, uv.min.y, uv.max.x, uv.max.y), (0.5, 0.25, 1.0, 0.5));
        let clamped = uv_of(ArtRect { x0: -20.0, y0: 0.0, x1: 500.0, y1: 10.0 }, Size::new(100.0, 100.0)).unwrap();
        assert_eq!((clamped.min.x, clamped.max.x), (0.0, 1.0));
        assert!(uv_of(ArtRect { x0: 5.0, y0: 5.0, x1: 5.0, y1: 9.0 }, Size::new(100.0, 100.0)).is_none(), "an empty box has no thumbnail");
        assert!(uv_of(ArtRect { x0: 0.0, y0: 0.0, x1: 1.0, y1: 1.0 }, Size::new(0.0, 0.0)).is_none());
    }
}
