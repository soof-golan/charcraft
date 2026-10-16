//! The drawing: ellipses, rounded boxes and gradients rasterised into layers.

#![allow(clippy::many_single_char_names, reason = "pixel and colour maths")]
#![allow(clippy::suboptimal_flops, reason = "pixel maths reads better as plain arithmetic")]

use anim_artwork::{ArtNode, ArtworkTree};
use anim_core::num::{f64_to_f32, round_f64_to_u32, unit_f32_to_u8};
use anim_core::{Point, Size};
use anim_render::image_from_straight_rgba;
use charcraft_puppet::Viseme;

use crate::{BodyShape, EyeStyle, HairStyle, HeadShape, Rgb, Spec};

/// Artwork width in pixels.
pub const WIDTH: u32 = 640;
/// Artwork height in pixels.
pub const HEIGHT: u32 = 720;

const WHITE: Rgb = [252, 252, 252];
const INK: Rgb = [28, 24, 30];
const MOUTH_IN: Rgb = [92, 22, 34];
const TONGUE: Rgb = [226, 112, 124];
const BLUSH: Rgb = [238, 140, 130];

fn f(v: u32) -> f32 {
    f64_to_f32(f64::from(v))
}

fn to_u(v: f32) -> u32 {
    round_f64_to_u32(f64::from(v).max(0.0))
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

fn mouth_layer(v: Viseme, cx: f32, cy: f32, lips: Rgb) -> ArtNode {
    let (rx, ry, teeth, tongue) = mouth_shape(v);
    let rect = (to_u(cx - rx - 4.0), to_u(cy - ry - 4.0), to_u(cx + rx + 4.0), to_u(cy + ry + 4.0));
    let outer = ellipse(cx, cy, rx + 3.0, ry + 3.0);
    let inner = ellipse(cx, cy, rx, ry);
    let top_row = cy - ry;
    let colour = move |x: f32, y: f32| {
        if inner(x, y) < 0.5 {
            return lips;
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

/// Radii of the eye white and the iris and pupil.
const fn eye_size(style: EyeStyle) -> (f32, f32, f32, f32) {
    match style {
        EyeStyle::Round => (36.0, 28.0, 15.0, 7.0),
        EyeStyle::Big => (42.0, 34.0, 22.0, 10.0),
        EyeStyle::Small => (24.0, 20.0, 11.0, 6.0),
    }
}

fn eye(spec: &Spec, side: &str, cx: f32, skin_shade: Rgb) -> ArtNode {
    let cy = 232.0;
    let (rx, ry, iris_r, pupil_r) = eye_size(spec.eyes);
    let ball = (to_u(cx - rx - 4.0), to_u(cy - ry - 4.0), to_u(cx + rx + 4.0), to_u(cy + ry + 4.0));
    let white = ellipse(cx, cy, rx, ry);
    let pupil_box = (to_u(cx - iris_r - 1.0), to_u(cy - iris_r - 1.0), to_u(cx + iris_r + 1.0), to_u(cy + iris_r + 1.0));
    let iris = ellipse(cx, cy, iris_r, iris_r);
    let pupil = ellipse(cx, cy, pupil_r, pupil_r);
    let glint = ellipse(cx - iris_r * 0.33, cy - iris_r * 0.4, iris_r * 0.23, iris_r * 0.23);
    let blink = ellipse(cx, cy + 4.0, rx + 2.0, 4.0);
    let iris_colour = spec.iris;
    let mut parts = vec![
        layer("White", ball, move |_, y| mix(WHITE, skin_shade, ((y - (cy - ry)) / 90.0).max(0.0)), white),
        layer(
            &format!("+ {side} Pupil"),
            pupil_box,
            move |x, y| {
                if glint(x, y) > 0.5 {
                    WHITE
                } else if pupil(x, y) > 0.5 {
                    INK
                } else {
                    mix(iris_colour, INK, ((y - cy).abs() / (iris_r * 2.0)).min(0.6))
                }
            },
            iris,
        ),
        hidden(layer(&format!("{side} Blink"), (to_u(cx - rx - 6.0), to_u(cy - 4.0), to_u(cx + rx + 6.0), to_u(cy + 14.0)), flat(INK), blink)),
    ];
    if spec.lashes {
        let (lx, ly) = (rx + 4.0, ry + 4.0);
        let arc = move |x: f32, y: f32| {
            let d = ((x - cx) / lx).hypot((y - cy) / ly);
            let band = (0.09 - (d - 0.98).abs()) * lx.min(ly) + 0.5;
            let edge = ((cy - 2.0 - y) * 2.0 + 0.5).clamp(0.0, 1.0);
            band.clamp(0.0, 1.0) * edge
        };
        parts.push(layer(&format!("{side} Eyelashes"), (to_u(cx - lx - 4.0), to_u(cy - ly - 8.0), to_u(cx + lx + 4.0), to_u(cy)), flat(INK), arc));
    }
    ArtNode::group(format!("{side} Eye"), parts)
}

fn brow(label: &str, cx: f32, colour: Rgb) -> ArtNode {
    let shape = round_rect(cx - 36.0, 176.0, cx + 36.0, 190.0, 7.0);
    layer(label, (to_u(cx - 38.0), 174, to_u(cx + 38.0), 192), flat(colour), shape)
}

/// Half the face width at the ears and the coverage of the face.
fn face(shape: HeadShape) -> (f32, Box<dyn Fn(f32, f32) -> f32>) {
    match shape {
        HeadShape::Round => (132.0, Box::new(ellipse(320.0, 250.0, 132.0, 154.0))),
        HeadShape::Oval => (112.0, Box::new(ellipse(320.0, 252.0, 112.0, 166.0))),
        HeadShape::Square => (136.0, Box::new(round_rect(184.0, 100.0, 456.0, 404.0, 52.0))),
    }
}

fn head(spec: &Spec) -> ArtNode {
    let skin = spec.skin;
    let skin_shade = mix(skin, [150, 60, 20], 0.22);
    let (half, face_cover) = face(spec.head);
    let mut parts: Vec<ArtNode> = Vec::new();

    let hair = spec.hair;
    match spec.hair_style {
        HairStyle::Bald => {}
        HairStyle::Short | HairStyle::Bun => parts.push(layer("Hair back", (140, 40, 500, 420), flat(hair), ellipse(320.0, 232.0, half + 22.0, 176.0))),
        HairStyle::Fringe => parts.push(layer("Hair back", (150, 30, 490, 430), flat(hair), ellipse(320.0, 230.0, 168.0, 196.0))),
        HairStyle::Long => parts.push(layer("Hair back", (130, 30, 510, 640), flat(hair), round_rect(140.0, 40.0, 500.0, 620.0, 120.0))),
    }
    if spec.hair_style == HairStyle::Bun {
        parts.push(layer("Bun", (260, 0, 380, 90), flat(hair), ellipse(320.0, 46.0, 48.0, 40.0)));
    }
    if spec.ears {
        let (left, right) = (ellipse(320.0 - half, 262.0, 22.0, 34.0), ellipse(320.0 + half, 262.0, 22.0, 34.0));
        parts.push(layer("Ears", (to_u(300.0 - half), 220, to_u(340.0 + half), 310), flat(skin_shade), move |x, y| left(x, y).max(right(x, y))));
    }
    parts.push(layer("Face", (176, 84, 464, 416), move |x, y| mix(skin, skin_shade, ((y - 250.0) / 300.0).max(0.0) + ((x - 320.0).abs() / 700.0)), face_cover));
    if spec.cheeks {
        let (left, right) = (ellipse(320.0 - half * 0.64, 316.0, 26.0, 16.0), ellipse(320.0 + half * 0.64, 316.0, 26.0, 16.0));
        parts.push(layer("Cheeks", (to_u(280.0 - half * 0.64), 298, to_u(360.0 + half * 0.64), 336), flat(BLUSH), move |x, y| {
            left(x, y).max(right(x, y)) * 0.55
        }));
    }
    parts.push(layer("Nose", (300, 284, 342, 310), flat(skin_shade), ellipse(320.0, 296.0, 15.0, 11.0)));
    parts.push(eye(spec, "Right", 258.0, skin_shade));
    parts.push(eye(spec, "Left", 382.0, skin_shade));
    parts.push(brow("+ Right Eyebrow", 258.0, hair));
    parts.push(brow("+Left Eyebrow", 382.0, hair));
    parts.push(ArtNode::group("mouth", Viseme::ALL.iter().map(|&v| mouth_layer(v, 320.0, 358.0, spec.lips)).collect()));
    if matches!(spec.hair_style, HairStyle::Fringe | HairStyle::Long) {
        let fringe = ellipse(320.0, 128.0, 142.0, 56.0);
        parts.push(layer("Fringe", (176, 62, 464, 196), flat(hair), move |x, y| fringe(x, y) * if y < 160.0 { 1.0 } else { 0.0 }));
    }
    ArtNode::group("Head", parts)
}

fn body(spec: &Spec) -> ArtNode {
    let (x0, x1) = match spec.body {
        BodyShape::Slim => (226.0, 414.0),
        BodyShape::Regular => (190.0, 450.0),
        BodyShape::Wide => (160.0, 480.0),
    };
    let skin_shade = mix(spec.skin, [150, 60, 20], 0.22);
    let (shirt, shirt_shade) = (spec.shirt, mix(spec.shirt, [0, 0, 0], 0.3));
    let torso = round_rect(x0, 470.0, x1, 760.0, 70.0);
    let neck = round_rect(284.0, 380.0, 356.0, 490.0, 16.0);
    ArtNode::group(
        "Body",
        vec![
            layer("Neck", (280, 376, 360, 494), flat(skin_shade), neck),
            layer(
                "Torso",
                (to_u(x0 - 4.0), 466, to_u(x1 + 4.0), HEIGHT),
                move |x, y| mix(shirt, shirt_shade, (y - 470.0) / 300.0 + (x - 320.0).abs() / 500.0),
                torso,
            ),
        ],
    )
}

/// The artwork of a character.
pub fn artwork(spec: &Spec) -> ArtworkTree {
    ArtworkTree { size: Size::new(f64::from(WIDTH), f64::from(HEIGHT)), root: ArtNode::group("", vec![body(spec), head(spec)]), warnings: Vec::new() }
}
