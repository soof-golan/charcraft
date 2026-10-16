//! The built-in demo puppet, drawn in code.
//!
//! A round-faced character with the tags Character Animator reads (`Head`, `Left Eye`, `+ Left Pupil`,
//! `Left Blink`, eyebrows, a `mouth` group with all fourteen mouth shapes, a `Body`). It lets `--smoke`,
//! `--screenshot` and a first launch work without any artwork on disk.

#![allow(clippy::many_single_char_names, reason = "pixel and colour maths")]

use anim_artwork::{ArtNode, ArtworkTree};
use anim_core::num::{f64_to_f32, unit_f32_to_u8};
use anim_core::{Point, Size};
use anim_render::image_from_straight_rgba;
use charcraft_puppet::{Puppet, Viseme};

/// Artwork width in pixels.
pub const WIDTH: u32 = 640;
/// Artwork height in pixels.
pub const HEIGHT: u32 = 720;

type Rgb = [u8; 3];

const SKIN: Rgb = [244, 196, 160];
const SKIN_SHADE: Rgb = [226, 166, 128];
const HAIR: Rgb = [74, 44, 30];
const SHIRT: Rgb = [58, 112, 200];
const SHIRT_SHADE: Rgb = [38, 80, 160];
const WHITE: Rgb = [252, 252, 252];
const INK: Rgb = [28, 24, 30];
const IRIS: Rgb = [60, 120, 90];
const LIP: Rgb = [176, 64, 76];
const MOUTH_IN: Rgb = [92, 22, 34];
const TONGUE: Rgb = [226, 112, 124];

fn f(v: u32) -> f32 {
    f64_to_f32(f64::from(v))
}

/// Coverage of an ellipse centred at `(cx, cy)` with radii `(rx, ry)`, antialiased over one pixel.
fn ellipse(cx: f32, cy: f32, rx: f32, ry: f32) -> impl Fn(f32, f32) -> f32 {
    move |x, y| {
        let d = ((x - cx) / rx).hypot((y - cy) / ry);
        ((1.0 - d) * rx.min(ry) + 0.5).clamp(0.0, 1.0)
    }
}

/// Coverage of a rounded rectangle.
fn round_rect(x0: f32, y0: f32, x1: f32, y1: f32, radius: f32) -> impl Fn(f32, f32) -> f32 {
    move |x, y| {
        let (cx, cy) = (f32::midpoint(x0, x1), f32::midpoint(y0, y1));
        let (hx, hy) = ((x1 - x0) / 2.0 - radius, (y1 - y0) / 2.0 - radius);
        let (qx, qy) = ((x - cx).abs() - hx, (y - cy).abs() - hy);
        let outside = qx.max(0.0).hypot(qy.max(0.0)) + qx.max(qy).min(0.0) - radius;
        (0.5 - outside).clamp(0.0, 1.0)
    }
}

fn mix(a: Rgb, b: Rgb, t: f32) -> Rgb {
    let t = t.clamp(0.0, 1.0);
    let ch = |i: usize| {
        let (p, q) = (f32::from(a.get(i).copied().unwrap_or(0)), f32::from(b.get(i).copied().unwrap_or(0)));
        unit_f32_to_u8((p + (q - p) * t) / 255.0)
    };
    [ch(0), ch(1), ch(2)]
}

/// One raster layer over the pixel box `x0..x1` by `y0..y1`: `colour` shades it and `shape` gives its coverage.
fn layer(name: &str, (x0, y0, x1, y1): (u32, u32, u32, u32), colour: impl Fn(f32, f32) -> Rgb, shape: impl Fn(f32, f32) -> f32) -> ArtNode {
    let (w, h) = (x1.saturating_sub(x0).max(1), y1.saturating_sub(y0).max(1));
    let pixels: Vec<u8> = (0..h)
        .flat_map(|j| {
            let (colour, shape) = (&colour, &shape);
            (0..w).flat_map(move |i| {
                let (x, y) = (f(x0.saturating_add(i)) + 0.5, f(y0.saturating_add(j)) + 0.5);
                let [r, g, b] = colour(x, y);
                [r, g, b, unit_f32_to_u8(shape(x, y))]
            })
        })
        .collect();
    let image = image_from_straight_rgba(w, h, &pixels).unwrap_or_else(|_| anim_display::Image { width: 1, height: 1, premultiplied_rgba: vec![0; 4] });
    ArtNode::raster(name, image, Point::new(f64::from(x0), f64::from(y0)))
}

fn flat(rgb: Rgb) -> impl Fn(f32, f32) -> Rgb {
    move |_, _| rgb
}

const fn hidden(mut node: ArtNode) -> ArtNode {
    node.visible = false;
    node
}

/// The mouth shapes: outline radii, whether teeth show and whether the tongue shows.
const fn mouth_shape(v: Viseme) -> (f32, f32, bool, bool) {
    match v {
        Viseme::Neutral => (30.0, 5.0, false, false),
        Viseme::Ah => (36.0, 34.0, true, true),
        Viseme::D => (32.0, 16.0, true, true),
        Viseme::Ee => (46.0, 12.0, true, false),
        Viseme::F => (34.0, 11.0, true, false),
        Viseme::L => (30.0, 20.0, true, true),
        Viseme::M => (32.0, 4.0, false, false),
        Viseme::Oh => (22.0, 28.0, false, true),
        Viseme::R => (26.0, 18.0, true, false),
        Viseme::S => (38.0, 8.0, true, false),
        Viseme::Uh => (28.0, 22.0, false, true),
        Viseme::WOo => (14.0, 16.0, false, false),
        Viseme::Smile => (48.0, 22.0, true, false),
        Viseme::Surprised => (26.0, 38.0, false, true),
    }
}

fn mouth_layer(v: Viseme, cx: f32, cy: f32) -> ArtNode {
    let (rx, ry, teeth, tongue) = mouth_shape(v);
    let bx = (cx - rx - 4.0, cy - ry - 4.0, cx + rx + 4.0, cy + ry + 4.0);
    let to_u = |v: f32| anim_core::num::round_f64_to_u32(f64::from(v).max(0.0));
    let rect = (to_u(bx.0), to_u(bx.1), to_u(bx.2), to_u(bx.3));
    let outer = ellipse(cx, cy, rx + 3.0, ry + 3.0);
    let inner = ellipse(cx, cy, rx, ry);
    let top_row = cy - ry;
    let colour = move |x: f32, y: f32| {
        if inner(x, y) < 0.5 {
            return LIP;
        }
        if teeth && y < top_row + ry * 0.55 {
            return WHITE;
        }
        if tongue && y > cy + ry * 0.2 {
            return TONGUE;
        }
        MOUTH_IN
    };
    let node = layer(v.label(), rect, colour, outer);
    if v == Viseme::Neutral { node } else { hidden(node) }
}

fn eye(side: &str, cx: f32) -> ArtNode {
    let cy = 232.0;
    let to_u = |v: f32| anim_core::num::round_f64_to_u32(f64::from(v).max(0.0));
    let ball = (to_u(cx - 40.0), to_u(cy - 32.0), to_u(cx + 40.0), to_u(cy + 32.0));
    let white = ellipse(cx, cy, 36.0, 28.0);
    let pupil_box = (to_u(cx - 16.0), to_u(cy - 16.0), to_u(cx + 16.0), to_u(cy + 16.0));
    let iris = ellipse(cx, cy, 15.0, 15.0);
    let pupil = ellipse(cx, cy, 7.0, 7.0);
    let glint = ellipse(cx - 5.0, cy - 6.0, 3.5, 3.5);
    let blink = ellipse(cx, cy + 4.0, 38.0, 4.0);
    ArtNode::group(
        format!("{side} Eye"),
        vec![
            layer("White", ball, |_, y| mix(WHITE, SKIN_SHADE, ((y - (cy - 28.0)) / 90.0).max(0.0)), white),
            layer(
                &format!("+ {side} Pupil"),
                pupil_box,
                move |x, y| {
                    if glint(x, y) > 0.5 {
                        WHITE
                    } else if pupil(x, y) > 0.5 {
                        INK
                    } else {
                        mix(IRIS, INK, ((y - cy).abs() / 30.0).min(0.6))
                    }
                },
                iris,
            ),
            hidden(layer(&format!("{side} Blink"), (to_u(cx - 42.0), to_u(cy - 4.0), to_u(cx + 42.0), to_u(cy + 14.0)), flat(INK), blink)),
        ],
    )
}

fn brow(label: &str, cx: f32) -> ArtNode {
    let to_u = |v: f32| anim_core::num::round_f64_to_u32(f64::from(v).max(0.0));
    let shape = round_rect(cx - 36.0, 176.0, cx + 36.0, 190.0, 7.0);
    layer(label, (to_u(cx - 38.0), 174, to_u(cx + 38.0), 192), flat(HAIR), shape)
}

fn head() -> ArtNode {
    let hair_back = ellipse(320.0, 230.0, 168.0, 196.0);
    let face = ellipse(320.0, 250.0, 132.0, 154.0);
    let ear_l = ellipse(188.0, 262.0, 22.0, 34.0);
    let ear_r = ellipse(452.0, 262.0, 22.0, 34.0);
    let nose = ellipse(320.0, 296.0, 15.0, 11.0);
    let cheek_l = ellipse(236.0, 316.0, 26.0, 16.0);
    let cheek_r = ellipse(404.0, 316.0, 26.0, 16.0);
    let fringe = ellipse(320.0, 128.0, 142.0, 56.0);
    let mouth = ArtNode::group("mouth", Viseme::ALL.iter().map(|&v| mouth_layer(v, 320.0, 358.0)).collect());
    ArtNode::group(
        "Head",
        vec![
            layer("Hair back", (150, 30, 490, 430), flat(HAIR), hair_back),
            layer("Ears", (160, 220, 480, 310), flat(SKIN_SHADE), move |x, y| ear_l(x, y).max(ear_r(x, y))),
            layer("Face", (186, 92, 454, 408), move |x, y| mix(SKIN, SKIN_SHADE, ((y - 250.0) / 300.0).max(0.0) + ((x - 320.0).abs() / 700.0)), face),
            layer("Cheeks", (200, 298, 440, 336), flat([238, 140, 130]), move |x, y| cheek_l(x, y).max(cheek_r(x, y)) * 0.55),
            layer("Nose", (300, 284, 342, 310), flat(SKIN_SHADE), nose),
            eye("Right", 258.0),
            eye("Left", 382.0),
            brow("+ Right Eyebrow", 258.0),
            brow("+Left Eyebrow", 382.0),
            mouth,
            layer("Fringe", (176, 62, 464, 196), flat(HAIR), move |x, y| fringe(x, y) * if y < 160.0 { 1.0 } else { 0.0 }),
        ],
    )
}

fn body() -> ArtNode {
    let torso = round_rect(190.0, 470.0, 450.0, 760.0, 70.0);
    let neck = round_rect(284.0, 380.0, 356.0, 490.0, 16.0);
    ArtNode::group(
        "Body",
        vec![
            layer("Neck", (280, 376, 360, 494), flat(SKIN_SHADE), neck),
            layer("Torso", (186, 466, 454, HEIGHT), move |x, y| mix(SHIRT, SHIRT_SHADE, (y - 470.0) / 300.0 + (x - 320.0).abs() / 500.0), torso),
        ],
    )
}

/// The demo character's artwork.
#[must_use]
pub fn artwork() -> ArtworkTree {
    ArtworkTree { size: Size::new(f64::from(WIDTH), f64::from(HEIGHT)), root: ArtNode::group("", vec![body(), head()]), warnings: Vec::new() }
}

/// The demo puppet.
///
/// # Errors
/// Whatever building a puppet from the artwork reports (it cannot fail for this artwork).
pub fn puppet() -> charcraft_puppet::Result<Puppet> {
    Puppet::from_artwork(&artwork())
}

#[cfg(test)]
mod tests {
    use charcraft_puppet::PuppetPose;

    use super::*;

    #[test]
    fn the_demo_puppet_has_a_rig() {
        let p = puppet().unwrap();
        assert_eq!(p.mouth_groups().len(), 1);
        assert_eq!(p.mouth_shapes(p.mouth_groups()[0]).len(), 14);
        assert!(p.find_path("Head/Left Eye").is_some());
        assert!(p.find_path("Head/Right Eye/Right Blink").is_some());
        let list = p.evaluate(&PuppetPose::neutral().with_viseme(Viseme::Ah));
        assert!(!list.nodes.is_empty());
    }
}
