//! The scene camera: pan, zoom and turn the whole stage, by hand, by keyframes, or by following a face.
//!
//! Scene space is centred on the middle of the frame, y down. The camera looks at the point `position` (relative
//! to the centre) at `zoom` times magnification and `rotation` degrees; the identity camera shows the stage as
//! it is. "Zoom on face" blends the camera towards the face of one track: at `amount` 1 the face sits in the
//! middle of the frame at `zoom` times magnification.

use anim_core::{Affine, Tick, Vec2};
use charcraft_puppet::{Puppet, PuppetPose};
use serde::{Deserialize, Serialize};

use crate::keyframes::{Lane, sanitize_lanes};
use crate::scene::Placement;

/// The camera fields keyframes can animate.
pub const TARGETS: [&str; 4] = ["position_x", "position_y", "zoom", "rotation"];
/// Smallest and largest zoom.
pub const ZOOM_RANGE: (f64, f64) = (0.05, 40.0);

/// Follows the face of a puppet track.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ZoomOnFace {
    /// Which puppet track's face.
    pub track: usize,
    /// How far the camera moves towards the face, 0 (not at all) to 1 (face centred).
    pub amount: f64,
    /// The magnification when `amount` is 1.
    pub zoom: f64,
}

impl Default for ZoomOnFace {
    fn default() -> Self {
        Self { track: 0, amount: 1.0, zoom: 2.0 }
    }
}

/// Where the camera looks.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Camera {
    /// The point looked at, in pixels from the centre of the frame.
    pub position: Vec2,
    /// Magnification (1 = the stage as it is).
    pub zoom: f64,
    /// Rotation in degrees, clockwise on screen.
    pub rotation: f64,
    /// Follow a face (`None`: the camera stays where the numbers put it).
    pub zoom_on_face: Option<ZoomOnFace>,
    /// Keyframes of `position_x`, `position_y`, `zoom` and `rotation`.
    pub lanes: Vec<Lane>,
}

impl Default for Camera {
    fn default() -> Self {
        Self { position: Vec2::ZERO, zoom: 1.0, rotation: 0.0, zoom_on_face: None, lanes: Vec::new() }
    }
}

/// The camera at one moment.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct View {
    /// The point looked at.
    pub position: Vec2,
    /// Magnification.
    pub zoom: f64,
    /// Degrees.
    pub rotation: f64,
}

impl View {
    /// The matrix from scene pixels to frame pixels for a frame centred on `centre`.
    #[must_use]
    pub fn matrix(&self, centre: Vec2) -> Affine {
        Affine::translate(centre) * Affine::rotate(self.rotation.to_radians()) * Affine::scale(self.zoom) * Affine::translate(Vec2::new(-(centre.x + self.position.x), -(centre.y + self.position.y)))
    }

    /// Whether this view shows the stage unchanged.
    #[must_use]
    pub fn is_identity(&self) -> bool {
        self.position == Vec2::ZERO && (self.zoom - 1.0).abs() < 1e-12 && self.rotation.abs() < 1e-12
    }
}

impl Camera {
    /// Whether the camera does anything at all.
    #[must_use]
    pub fn is_neutral(&self) -> bool {
        self.zoom_on_face.is_none() && self.lanes.is_empty() && self.position == Vec2::ZERO && (self.zoom - 1.0).abs() < 1e-12 && self.rotation.abs() < 1e-12
    }

    /// The camera at scene time `t`. `face` is the scene-space position of the followed face (relative to the
    /// middle of the frame), when it is known.
    #[must_use]
    pub fn view_at(&self, t: Tick, face: Option<Vec2>) -> View {
        let lane = |name: &str, base: f64| self.lanes.iter().find(|l| l.target == name).and_then(|l| l.value_at(t)).unwrap_or(base);
        let mut view = View { position: Vec2::new(lane("position_x", self.position.x), lane("position_y", self.position.y)), zoom: lane("zoom", self.zoom), rotation: lane("rotation", self.rotation) };
        if let (Some(follow), Some(face)) = (self.zoom_on_face, face) {
            let a = follow.amount.clamp(0.0, 1.0);
            view.position = Vec2::new(view.position.x + (face.x - view.position.x) * a, view.position.y + (face.y - view.position.y) * a);
            view.zoom *= 1.0 + (follow.zoom - 1.0) * a;
        }
        view.zoom = view.zoom.clamp(ZOOM_RANGE.0, ZOOM_RANGE.1);
        view
    }

    /// Repairs a camera that came from outside.
    ///
    /// # Errors
    /// [`Error::TooLarge`](crate::Error::TooLarge).
    pub fn sanitize(&mut self) -> crate::Result<()> {
        let fix = |v: f64, d: f64| if v.is_finite() { v.clamp(-1.0e7, 1.0e7) } else { d };
        self.position = Vec2::new(fix(self.position.x, 0.0), fix(self.position.y, 0.0));
        self.zoom = fix(self.zoom, 1.0).clamp(ZOOM_RANGE.0, ZOOM_RANGE.1);
        self.rotation = fix(self.rotation, 0.0);
        if let Some(follow) = self.zoom_on_face.as_mut() {
            follow.amount = fix(follow.amount, 1.0).clamp(0.0, 1.0);
            follow.zoom = fix(follow.zoom, 2.0).clamp(ZOOM_RANGE.0, ZOOM_RANGE.1);
        }
        self.lanes.retain(|l| TARGETS.contains(&l.target.as_str()));
        sanitize_lanes(&mut self.lanes)
    }
}

/// The scene-space position, relative to the middle of the frame, of the face of a puppet standing as
/// `placement` says, in `pose`: the centre of the group tagged `Head` (the whole puppet when it has none).
#[must_use]
pub fn face_point(puppet: &Puppet, pose: &PuppetPose, placement: &Placement, skin: Vec2, handle: Vec2) -> Option<Vec2> {
    let head = puppet.with_tag(charcraft_puppet::tags::ids::HEAD).into_iter().next();
    let (local, matrix) = if let Some((id, layer)) = head.and_then(|id| puppet.layer(id).map(|l| (id, l))) {
        let centre = layer.bounds.map_or(layer.origin, |b| b.center());
        (centre, puppet.group_matrix(id, pose))
    } else {
        let b = puppet.rest_bounds()?;
        (b.center(), Affine::IDENTITY)
    };
    let posed = matrix * local;
    // The placement matrix is for a frame centred on the origin, which is what "relative to the middle" means.
    let p = placement.matrix(Vec2::ZERO, skin, handle) * posed;
    p.x.is_finite().then_some(Vec2::new(p.x, p.y))
}

#[cfg(test)]
mod tests {
    use anim_core::Point;

    use super::*;

    #[test]
    fn the_neutral_camera_is_the_identity() {
        let view = Camera::default().view_at(Tick::ZERO, None);
        assert!(view.is_identity());
        let p = view.matrix(Vec2::new(100.0, 50.0)) * Point::new(7.0, 9.0);
        assert!((p.x - 7.0).abs() < 1e-9 && (p.y - 9.0).abs() < 1e-9);
    }

    #[test]
    fn zoom_scales_about_the_looked_at_point() {
        let camera = Camera { position: Vec2::new(40.0, 0.0), zoom: 2.0, ..Camera::default() };
        let view = camera.view_at(Tick::ZERO, None);
        let centre = Vec2::new(100.0, 50.0);
        // The looked-at point lands in the middle of the frame.
        let p = view.matrix(centre) * Point::new(140.0, 50.0);
        assert!((p.x - 100.0).abs() < 1e-9 && (p.y - 50.0).abs() < 1e-9);
        // One pixel to its right lands two pixels from the middle.
        let q = view.matrix(centre) * Point::new(141.0, 50.0);
        assert!((q.x - 102.0).abs() < 1e-9);
    }

    #[test]
    fn zoom_on_face_blends_towards_the_face() {
        let mut camera = Camera { zoom_on_face: Some(ZoomOnFace { track: 0, amount: 1.0, zoom: 3.0 }), ..Camera::default() };
        let face = Some(Vec2::new(-30.0, 20.0));
        let view = camera.view_at(Tick::ZERO, face);
        assert_eq!(view.position, Vec2::new(-30.0, 20.0));
        assert!((view.zoom - 3.0).abs() < 1e-9);
        camera.zoom_on_face = Some(ZoomOnFace { track: 0, amount: 0.5, zoom: 3.0 });
        let half = camera.view_at(Tick::ZERO, face);
        assert_eq!(half.position, Vec2::new(-15.0, 10.0));
        assert!((half.zoom - 2.0).abs() < 1e-9);
        assert!(camera.view_at(Tick::ZERO, None).is_identity(), "no face known: the camera stays put");
    }

    #[test]
    fn keyframes_move_the_camera() {
        let mut camera = Camera::default();
        let mut lane = Lane::new("zoom");
        lane.set_key(Tick::from_seconds_f64(0.0), 1.0, None).unwrap();
        lane.set_key(Tick::from_seconds_f64(2.0), 3.0, None).unwrap();
        camera.lanes.push(lane);
        assert!((camera.view_at(Tick::from_seconds_f64(1.0), None).zoom - 2.0).abs() < 1e-9);
    }

    #[test]
    fn hostile_cameras_are_repaired() {
        let mut camera = Camera { position: Vec2::new(f64::NAN, 1.0), zoom: f64::INFINITY, rotation: f64::NAN, ..Camera::default() };
        camera.lanes.push(Lane::new("nonsense"));
        camera.sanitize().unwrap();
        assert!(camera.lanes.is_empty());
        assert!(camera.zoom.is_finite() && camera.position.x == 0.0);
    }
}
