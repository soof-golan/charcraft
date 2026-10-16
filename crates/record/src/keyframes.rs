//! Parameter keyframes: a value that changes over the scene by hand-set keys with an ease between them.
//!
//! A [`Lane`] animates one named parameter of a puppet track (its placement: position, scale, rotation, opacity;
//! or a numeric parameter of one of its behaviors) or of the scene camera. Between two keys the value moves
//! along the [`Ease`] of the earlier key; before the first and after the last key it holds.

use anim_core::Tick;
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

/// Most keys one lane keeps.
pub const MAX_KEYS: usize = 100_000;
/// Most lanes a track (or the camera) keeps.
pub const MAX_LANES: usize = 256;
/// Longest lane target name kept.
pub const MAX_TARGET: usize = 128;

/// How the value moves from a key to the next one.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Ease {
    /// Constant speed.
    #[default]
    Linear,
    /// Jump to the next value at the next key.
    Hold,
    /// Slow start.
    EaseIn,
    /// Slow end.
    EaseOut,
    /// Slow start and end.
    EaseInOut,
    /// A CSS-style cubic Bezier through (0,0) and (1,1) with these two control points.
    Bezier {
        /// First control point x, 0 to 1.
        x1: f64,
        /// First control point y.
        y1: f64,
        /// Second control point x, 0 to 1.
        x2: f64,
        /// Second control point y.
        y2: f64,
    },
}

impl Ease {
    /// The presets by name (`linear`, `hold`, `ease_in`, `ease_out`, `ease_in_out`).
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        match name.trim().to_ascii_lowercase().replace([' ', '-'], "_").as_str() {
            "linear" => Some(Self::Linear),
            "hold" | "step" => Some(Self::Hold),
            "ease_in" => Some(Self::EaseIn),
            "ease_out" => Some(Self::EaseOut),
            "ease_in_out" | "ease" => Some(Self::EaseInOut),
            _ => None,
        }
    }

    /// The name of a preset (`bezier` for a custom curve).
    #[must_use]
    pub const fn label(&self) -> &'static str {
        match self {
            Self::Linear => "linear",
            Self::Hold => "hold",
            Self::EaseIn => "ease_in",
            Self::EaseOut => "ease_out",
            Self::EaseInOut => "ease_in_out",
            Self::Bezier { .. } => "bezier",
        }
    }

    /// The control points of the curve, when it has a Bezier form.
    const fn bezier(&self) -> Option<[f64; 4]> {
        match *self {
            Self::EaseIn => Some([0.42, 0.0, 1.0, 1.0]),
            Self::EaseOut => Some([0.0, 0.0, 0.58, 1.0]),
            Self::EaseInOut => Some([0.42, 0.0, 0.58, 1.0]),
            Self::Bezier { x1, y1, x2, y2 } => Some([x1, y1, x2, y2]),
            Self::Linear | Self::Hold => None,
        }
    }

    /// Maps linear progress `u` (0 to 1) to eased progress.
    #[must_use]
    pub fn apply(&self, u: f64) -> f64 {
        let u = if u.is_finite() { u.clamp(0.0, 1.0) } else { 0.0 };
        match self {
            Self::Linear => u,
            Self::Hold => 0.0,
            _ => self.bezier().map_or(u, |[x1, y1, x2, y2]| bezier_y(u, x1.clamp(0.0, 1.0), y1, x2.clamp(0.0, 1.0), y2)),
        }
    }

    /// Repairs non-finite control points.
    pub fn sanitize(&mut self) {
        if let Self::Bezier { x1, y1, x2, y2 } = self {
            let fix = |v: f64| if v.is_finite() { v.clamp(-4.0, 5.0) } else { 0.0 };
            *x1 = fix(*x1).clamp(0.0, 1.0);
            *x2 = fix(*x2).clamp(0.0, 1.0);
            *y1 = fix(*y1);
            *y2 = fix(*y2);
        }
    }
}

/// The y of a cubic Bezier (0,0)-(x1,y1)-(x2,y2)-(1,1) at x = `u`, by bisection (deterministic, no iteration
/// count that depends on the machine).
fn bezier_y(u: f64, x1: f64, y1: f64, x2: f64, y2: f64) -> f64 {
    let at = |t: f64, a: f64, b: f64| {
        let s = 1.0 - t;
        3.0 * s * s * t * a + 3.0 * s * t * t * b + t * t * t
    };
    let (mut lo, mut hi) = (0.0_f64, 1.0_f64);
    for _ in 0..40 {
        let mid = f64::midpoint(lo, hi);
        if at(mid, x1, x2) < u {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    at(f64::midpoint(lo, hi), y1, y2)
}

/// One key: a value at a scene time, and the ease towards the next key.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Key {
    /// Scene time.
    pub t: Tick,
    /// The value.
    pub value: f64,
    /// How the value moves on to the next key.
    #[serde(default)]
    pub ease: Ease,
}

/// The keys of one parameter.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Lane {
    /// What is animated: a placement field (`position_x`, `position_y`, `anchor_x`, `anchor_y`, `scale_x`,
    /// `scale_y`, `rotation`, `opacity`), a camera field (`position_x`, `position_y`, `zoom`, `rotation`) or a
    /// behavior parameter of a track (`behavior.<slot>.<parameter>`).
    pub target: String,
    /// The keys, in time order, one per time.
    pub keys: Vec<Key>,
}

impl Lane {
    /// An empty lane.
    #[must_use]
    pub fn new(target: &str) -> Self {
        Self { target: target.to_owned(), keys: Vec::new() }
    }

    /// The value at scene time `t`; `None` without keys.
    #[must_use]
    pub fn value_at(&self, t: Tick) -> Option<f64> {
        let next_index = self.keys.partition_point(|k| k.t.0 <= t.0);
        let prev = next_index.checked_sub(1).and_then(|i| self.keys.get(i));
        let next = self.keys.get(next_index);
        match (prev, next) {
            (Some(from), Some(to)) => {
                let span = to.t.0.saturating_sub(from.t.0);
                let along = if span <= 0 { 1.0 } else { crate::take::ratio(t.0.saturating_sub(from.t.0), span) };
                let eased = from.ease.apply(along);
                Some(from.value + (to.value - from.value) * eased)
            }
            (Some(from), None) => Some(from.value),
            (None, Some(to)) => Some(to.value),
            (None, None) => None,
        }
    }

    /// The index of the key at exactly `t`.
    #[must_use]
    pub fn key_at(&self, t: Tick) -> Option<usize> {
        self.keys.binary_search_by_key(&t.0, |k| k.t.0).ok()
    }

    /// Puts a key at `t` (replacing the one there). Returns its index.
    ///
    /// # Errors
    /// [`Error::TooLarge`].
    pub fn set_key(&mut self, t: Tick, value: f64, ease: Option<Ease>) -> Result<usize> {
        match self.keys.binary_search_by_key(&t.0, |k| k.t.0) {
            Ok(i) => {
                if let Some(key) = self.keys.get_mut(i) {
                    key.value = value;
                    if let Some(e) = ease {
                        key.ease = e;
                    }
                }
                Ok(i)
            }
            Err(i) => {
                if self.keys.len() >= MAX_KEYS {
                    return Err(Error::TooLarge { what: "a keyframe lane", limit: MAX_KEYS });
                }
                self.keys.insert(i, Key { t, value, ease: ease.unwrap_or_default() });
                Ok(i)
            }
        }
    }

    /// Moves key `index` to time `t` (a key already at `t` is replaced). Returns the key's new index.
    pub fn move_key(&mut self, index: usize, t: Tick) -> Option<usize> {
        if index >= self.keys.len() {
            return None;
        }
        let mut key = self.keys.remove(index);
        key.t = t;
        match self.keys.binary_search_by_key(&t.0, |k| k.t.0) {
            Ok(i) => {
                if let Some(slot) = self.keys.get_mut(i) {
                    *slot = key;
                }
                Some(i)
            }
            Err(i) => {
                self.keys.insert(i, key);
                Some(i)
            }
        }
    }

    /// Sorts, de-duplicates and repairs the keys of a lane that came from a file.
    ///
    /// # Errors
    /// [`Error::TooLarge`].
    pub fn sanitize(&mut self) -> Result<()> {
        if self.keys.len() > MAX_KEYS {
            return Err(Error::TooLarge { what: "a keyframe lane", limit: MAX_KEYS });
        }
        if self.target.chars().count() > MAX_TARGET {
            self.target = self.target.chars().take(MAX_TARGET).collect();
        }
        self.keys.retain(|k| k.value.is_finite());
        for key in &mut self.keys {
            key.ease.sanitize();
            key.value = key.value.clamp(-1.0e7, 1.0e7);
        }
        self.keys.sort_by_key(|k| k.t);
        self.keys.dedup_by_key(|k| k.t);
        Ok(())
    }
}

/// Sanitizes a list of lanes: drops empty duplicates, caps the count.
///
/// # Errors
/// [`Error::TooLarge`].
pub fn sanitize_lanes(lanes: &mut Vec<Lane>) -> Result<()> {
    if lanes.len() > MAX_LANES {
        return Err(Error::TooLarge { what: "the lane list", limit: MAX_LANES });
    }
    for lane in lanes.iter_mut() {
        lane.sanitize()?;
    }
    lanes.retain(|l| !l.target.is_empty());
    Ok(())
}

/// The lane for `target` (created when absent).
///
/// # Errors
/// [`Error::TooLarge`] when the list is full.
pub fn lane_mut<'a>(lanes: &'a mut Vec<Lane>, target: &str) -> Result<&'a mut Lane> {
    let index = if let Some(i) = lanes.iter().position(|l| l.target == target) {
        i
    } else {
        if lanes.len() >= MAX_LANES {
            return Err(Error::TooLarge { what: "the lane list", limit: MAX_LANES });
        }
        lanes.push(Lane::new(target));
        lanes.len().saturating_sub(1)
    };
    lanes.get_mut(index).ok_or_else(|| Error::invalid("lane vanished"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seconds(s: f64) -> Tick {
        Tick::from_seconds_f64(s)
    }

    fn lane() -> Lane {
        let mut lane = Lane::new("position_x");
        lane.set_key(seconds(1.0), 0.0, None).unwrap();
        lane.set_key(seconds(3.0), 100.0, None).unwrap();
        lane
    }

    #[test]
    fn values_hold_before_and_after_and_interpolate_between() {
        let l = lane();
        assert_eq!(l.value_at(seconds(0.0)), Some(0.0));
        assert!((l.value_at(seconds(2.0)).unwrap() - 50.0).abs() < 1e-9);
        assert_eq!(l.value_at(seconds(9.0)), Some(100.0));
        assert_eq!(Lane::new("x").value_at(seconds(1.0)), None);
    }

    #[test]
    fn eases_shape_the_motion() {
        let mut l = lane();
        l.keys[0].ease = Ease::EaseIn;
        let quarter = l.value_at(seconds(1.5)).unwrap();
        assert!(quarter < 25.0, "ease in starts slowly: {quarter}");
        l.keys[0].ease = Ease::EaseOut;
        assert!(l.value_at(seconds(1.5)).unwrap() > 25.0);
        l.keys[0].ease = Ease::Hold;
        assert_eq!(l.value_at(seconds(2.9)), Some(0.0));
        assert_eq!(l.value_at(seconds(3.0)), Some(100.0));
        l.keys[0].ease = Ease::EaseInOut;
        assert!((l.value_at(seconds(2.0)).unwrap() - 50.0).abs() < 1e-6, "symmetric about the middle");
        l.keys[0].ease = Ease::Bezier { x1: 0.0, y1: 0.0, x2: 1.0, y2: 1.0 };
        assert!((l.value_at(seconds(2.0)).unwrap() - 50.0).abs() < 1e-6);
    }

    #[test]
    fn keys_are_set_moved_and_replaced() {
        let mut l = lane();
        assert_eq!(l.set_key(seconds(2.0), 5.0, Some(Ease::Hold)).unwrap(), 1);
        assert_eq!(l.keys.len(), 3);
        assert_eq!(l.set_key(seconds(2.0), 6.0, None).unwrap(), 1);
        assert_eq!(l.keys.len(), 3);
        assert_eq!(l.keys[1].ease, Ease::Hold, "an update without an ease keeps it");
        assert_eq!(l.move_key(1, seconds(3.0)), Some(1));
        assert_eq!(l.keys.len(), 2, "moving onto a key replaces it");
        assert_eq!(l.move_key(9, seconds(0.0)), None);
        assert_eq!(l.key_at(seconds(3.0)), Some(1));
    }

    #[test]
    fn hostile_lanes_are_repaired() {
        let mut l = Lane::new("x");
        l.keys = vec![
            Key { t: seconds(2.0), value: 1.0, ease: Ease::Bezier { x1: f64::NAN, y1: 9.0, x2: 7.0, y2: f64::INFINITY } },
            Key { t: seconds(1.0), value: f64::NAN, ease: Ease::Linear },
            Key { t: seconds(2.0), value: 5.0, ease: Ease::Linear },
        ];
        l.sanitize().unwrap();
        assert_eq!(l.keys.len(), 1);
        assert!(l.value_at(seconds(5.0)).unwrap().is_finite());
        assert!(Ease::Bezier { x1: 2.0, y1: -9.0, x2: -3.0, y2: 9.0 }.apply(0.5).is_finite());
    }

    #[test]
    fn eases_parse_by_name() {
        assert_eq!(Ease::parse("Ease In-Out"), Some(Ease::EaseInOut));
        assert_eq!(Ease::parse("hold"), Some(Ease::Hold));
        assert_eq!(Ease::parse("wobble"), None);
    }
}
