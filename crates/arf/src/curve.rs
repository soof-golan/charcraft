//! Curves: a channel's value over time as a run of cubic Bezier segments.
//!
//! One segment is four control points `(time, value)`. A held value is a flat segment, a jump is a vertical
//! segment (all four times equal) and a measured signal such as the voice's amplitude is a smooth run of segments.
//! Consecutive segments share their end point: the next one starts where the previous ended.

/// Four `(time, value)` control points of a cubic Bezier.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Segment {
    /// Control point times, seconds.
    pub t: [f64; 4],
    /// Control point values.
    pub v: [f64; 4],
}

/// Bisection steps when solving a segment for a time (the result is exact to well below a nanosecond).
const SOLVE_STEPS: u32 = 48;

fn bernstein(u: f64) -> [f64; 4] {
    let w = 1.0 - u;
    [w * w * w, 3.0 * u * w * w, 3.0 * u * u * w, u * u * u]
}

fn mix(points: &[f64; 4], b: &[f64; 4]) -> f64 {
    points.iter().zip(b).map(|(p, w)| p * w).sum()
}

fn lerp(a: f64, b: f64, u: f64) -> f64 {
    a + (b - a) * u
}

impl Segment {
    /// A value held from `t0` to `t1`.
    #[must_use]
    pub fn hold(t0: f64, t1: f64, value: f64) -> Self {
        Self { t: thirds(t0, t1), v: [value; 4] }
    }

    /// A straight ramp.
    #[must_use]
    pub fn line(t0: f64, v0: f64, t1: f64, v1: f64) -> Self {
        Self { t: thirds(t0, t1), v: [v0, lerp(v0, v1, 1.0 / 3.0), lerp(v0, v1, 2.0 / 3.0), v1] }
    }

    /// A jump at `t` from `before` to `after`.
    #[must_use]
    pub const fn step(t: f64, before: f64, after: f64) -> Self {
        Self { t: [t; 4], v: [before, before, after, after] }
    }

    /// When the segment starts.
    #[must_use]
    pub const fn start(&self) -> f64 {
        self.t[0]
    }

    /// When the segment ends.
    #[must_use]
    pub const fn end(&self) -> f64 {
        self.t[3]
    }

    /// The point of the segment at curve parameter `u`.
    #[must_use]
    pub fn point(&self, u: f64) -> (f64, f64) {
        let b = bernstein(u);
        (mix(&self.t, &b), mix(&self.v, &b))
    }

    /// The parameter at which the segment is at time `time` (control times are expected not to go backwards).
    fn parameter_at(&self, time: f64) -> f64 {
        let (mut lo, mut hi) = (0.0_f64, 1.0_f64);
        for _ in 0..SOLVE_STEPS {
            let mid = f64::midpoint(lo, hi);
            if mix(&self.t, &bernstein(mid)) < time {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        f64::midpoint(lo, hi)
    }

    /// The value at `time`: the first value before the segment, the last after it.
    #[must_use]
    pub fn value_at(&self, time: f64) -> f64 {
        if time.is_nan() {
            return self.v[0];
        }
        // A jump (all times equal) is its new value from the moment it happens.
        if time >= self.t[3] {
            return self.v[3];
        }
        if time <= self.t[0] {
            return self.v[0];
        }
        if self.v.iter().all(|&y| y.to_bits() == self.v[0].to_bits()) {
            return self.v[0];
        }
        self.point(self.parameter_at(time)).1
    }

    /// Splits at `time` (de Casteljau). Both halves keep the shape.
    #[must_use]
    pub fn split_at(&self, time: f64) -> (Self, Self) {
        let u = self.parameter_at(time).clamp(0.0, 1.0);
        let cast = |p: &[f64; 4]| {
            let a = [lerp(p[0], p[1], u), lerp(p[1], p[2], u), lerp(p[2], p[3], u)];
            let b = [lerp(a[0], a[1], u), lerp(a[1], a[2], u)];
            let c = lerp(b[0], b[1], u);
            ([p[0], a[0], b[0], c], [c, b[1], a[2], p[3]])
        };
        let (t1, t2) = cast(&self.t);
        let (v1, v2) = cast(&self.v);
        (Self { t: t1, v: v1 }, Self { t: t2, v: v2 })
    }
}

fn thirds(t0: f64, t1: f64) -> [f64; 4] {
    [t0, lerp(t0, t1, 1.0 / 3.0), lerp(t0, t1, 2.0 / 3.0), t1]
}

/// A channel's value over time.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Curve {
    /// The segments, in time order.
    pub segments: Vec<Segment>,
}

impl Curve {
    /// A curve from segments already in time order.
    #[must_use]
    pub const fn new(segments: Vec<Segment>) -> Self {
        Self { segments }
    }

    /// Whether the curve has no segment.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.segments.is_empty()
    }

    /// When the curve starts.
    #[must_use]
    pub fn start(&self) -> Option<f64> {
        self.segments.first().map(Segment::start)
    }

    /// When the curve ends.
    #[must_use]
    pub fn end(&self) -> Option<f64> {
        self.segments.last().map(Segment::end)
    }

    /// The value at `time`: held before the first segment and after the last, and at a jump the value after it.
    /// `None` for an empty curve.
    #[must_use]
    pub fn value_at(&self, time: f64) -> Option<f64> {
        let after = self.segments.partition_point(|s| s.start() <= time);
        let segment = after.checked_sub(1).and_then(|i| self.segments.get(i)).or_else(|| self.segments.first())?;
        Some(segment.value_at(time))
    }

    /// The parts of the curve in the window from `t0` up to `t1` (and including `t1` when `closed`), cutting
    /// segments that cross an edge. A jump exactly at `t0` belongs to this window, one at `t1` to the next.
    #[must_use]
    pub fn slice(&self, t0: f64, t1: f64, closed: bool) -> Vec<Segment> {
        let mut out = Vec::new();
        for segment in &self.segments {
            let after_start = segment.end() > t0 || segment.start() >= t0;
            let before_end = segment.start() < t1 || (closed && segment.start() <= t1);
            if !after_start || !before_end {
                continue;
            }
            let mut piece = *segment;
            if piece.start() < t0 && piece.end() > t0 {
                piece = piece.split_at(t0).1;
            }
            if piece.end() > t1 && piece.start() < t1 {
                piece = piece.split_at(t1).0;
            }
            out.push(piece);
        }
        out
    }

    /// A curve through `(time, value)` samples, ramping straight between them (`step` holds each value until the
    /// next sample instead).
    #[must_use]
    pub fn from_samples(samples: &[(f64, f64)], step: bool) -> Self {
        let mut segments = Vec::new();
        for (&(t0, v0), &(t1, v1)) in samples.iter().zip(samples.iter().skip(1)) {
            if step {
                segments.push(Segment::hold(t0, t1, v0));
                if v1.to_bits() != v0.to_bits() {
                    segments.push(Segment::step(t1, v0, v1));
                }
            } else {
                segments.push(Segment::line(t0, v0, t1, v1));
            }
        }
        if segments.is_empty()
            && let Some(&(t, v)) = samples.first()
        {
            segments.push(Segment::hold(t, t, v));
        }
        Self { segments }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn holds_steps_and_ramps() {
        let curve = Curve::from_samples(&[(0.0, 1.0), (1.0, 3.0), (2.0, 3.0), (3.0, 0.0)], true);
        assert_eq!(curve.value_at(-1.0), Some(1.0));
        assert_eq!(curve.value_at(0.5), Some(1.0));
        assert_eq!(curve.value_at(1.0), Some(3.0), "at a jump the new value");
        assert_eq!(curve.value_at(2.5), Some(3.0));
        assert_eq!(curve.value_at(3.0), Some(0.0));
        assert_eq!(curve.value_at(9.0), Some(0.0));
        let ramp = Curve::from_samples(&[(0.0, 0.0), (2.0, 4.0)], false);
        let mid = ramp.value_at(1.0).unwrap();
        assert!((mid - 2.0).abs() < 1e-9, "{mid}");
        assert_eq!(Curve::default().value_at(1.0), None);
    }

    #[test]
    fn splitting_keeps_the_shape() {
        let s = Segment { t: [0.0, 0.2, 0.9, 1.0], v: [0.0, 5.0, -3.0, 1.0] };
        let (a, b) = s.split_at(0.4);
        assert!((a.end() - 0.4).abs() < 1e-9 && (b.start() - 0.4).abs() < 1e-9);
        for time in [0.1, 0.3, 0.5, 0.8] {
            let whole = s.value_at(time);
            let part = if time < 0.4 { a.value_at(time) } else { b.value_at(time) };
            assert!((whole - part).abs() < 1e-6, "{time}: {whole} vs {part}");
        }
    }

    #[test]
    fn slicing_cuts_at_the_edges() {
        let curve = Curve::from_samples(&[(0.0, 0.0), (10.0, 10.0)], false);
        let pieces = curve.slice(2.0, 5.0, false);
        assert_eq!(pieces.len(), 1);
        assert!((pieces[0].start() - 2.0).abs() < 1e-9 && (pieces[0].end() - 5.0).abs() < 1e-9);
        assert!((pieces[0].value_at(3.5) - 3.5).abs() < 1e-6);
    }
}
