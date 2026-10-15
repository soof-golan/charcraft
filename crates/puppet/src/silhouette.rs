//! Outlines of rendered artwork: the polygon a mesh is built from.

// Grid coordinates are bounded by `MAX_GRID`, far below the range where `i32`/`i64` arithmetic could overflow.
#![allow(clippy::arithmetic_side_effects, reason = "coordinates are bounded by MAX_GRID")]

use std::collections::BTreeMap;

type Corner = (i32, i32);

/// Largest grid side accepted.
pub const MAX_GRID: usize = 8192;

/// The boundary of the largest filled region of a pixel grid, as a closed polygon of grid corners (the last
/// corner is not repeated). `mask` is row-major, `width * height` cells. `None` when nothing is filled.
///
/// Diagonal pinches are split into separate loops, so each loop is a simple polygon.
#[must_use]
pub fn largest_contour(mask: &[bool], width: usize, height: usize) -> Option<Vec<Corner>> {
    if width > MAX_GRID || height > MAX_GRID {
        return None;
    }
    let w = i32::try_from(width).ok()?;
    let h = i32::try_from(height).ok()?;
    let filled = |x: i32, y: i32| -> bool {
        if x < 0 || y < 0 || x >= w || y >= h {
            return false;
        }
        let (Ok(xu), Ok(yu)) = (usize::try_from(x), usize::try_from(y)) else { return false };
        mask.get(yu.saturating_mul(width).saturating_add(xu)).copied().unwrap_or(false)
    };

    // Clockwise (on screen) boundary edges: the filled cell is on the right of the walking direction.
    let mut edges: BTreeMap<Corner, Vec<Corner>> = BTreeMap::new();
    for y in 0..h {
        for x in 0..w {
            if !filled(x, y) {
                continue;
            }
            let mut add = |from: Corner, to: Corner| edges.entry(from).or_default().push(to);
            if !filled(x, y - 1) {
                add((x, y), (x + 1, y));
            }
            if !filled(x + 1, y) {
                add((x + 1, y), (x + 1, y + 1));
            }
            if !filled(x, y + 1) {
                add((x + 1, y + 1), (x, y + 1));
            }
            if !filled(x - 1, y) {
                add((x, y + 1), (x, y));
            }
        }
    }

    let mut best: Option<(i64, Vec<Corner>)> = None;
    let mut budget = edges.values().map(Vec::len).sum::<usize>().saturating_add(1);
    while let Some((&start, _)) = edges.iter().next() {
        let mut ring = vec![start];
        let mut heading: Option<Corner> = None;
        let mut at = start;
        loop {
            budget = budget.saturating_sub(1);
            let Some(options) = edges.get_mut(&at) else { break };
            // Prefer the sharpest right turn so touching corners split into separate loops.
            let pick = heading.map_or(0, |d| {
                options
                    .iter()
                    .enumerate()
                    .max_by_key(|&(_, &to)| {
                        let nd = (to.0 - at.0, to.1 - at.1);
                        d.0 * nd.1 - d.1 * nd.0
                    })
                    .map_or(0, |(i, _)| i)
            });
            let to = options.swap_remove(pick.min(options.len().saturating_sub(1)));
            if options.is_empty() {
                edges.remove(&at);
            }
            heading = Some((to.0 - at.0, to.1 - at.1));
            at = to;
            if at == start || budget == 0 {
                break;
            }
            ring.push(at);
        }
        let area = signed_area(&ring).abs();
        if best.as_ref().is_none_or(|(a, _)| area > *a) {
            best = Some((area, ring));
        }
        if budget == 0 {
            break;
        }
    }
    best.map(|(_, ring)| ring).filter(|ring| ring.len() >= 3)
}

fn signed_area(ring: &[Corner]) -> i64 {
    let mut sum = 0_i64;
    for (i, a) in ring.iter().enumerate() {
        let b = ring.get((i + 1) % ring.len().max(1)).unwrap_or(a);
        sum = sum.saturating_add(i64::from(a.0) * i64::from(b.1) - i64::from(b.0) * i64::from(a.1));
    }
    sum / 2
}

/// Douglas-Peucker on a closed ring of grid corners; `tolerance` is in cells.
#[must_use]
pub fn simplify(ring: &[Corner], tolerance: f64) -> Vec<Corner> {
    if ring.len() < 4 {
        return ring.to_vec();
    }
    // Split at the corner farthest from the first so both halves are open polylines.
    let first = ring.first().copied().unwrap_or((0, 0));
    let far = ring.iter().enumerate().max_by_key(|&(_, &c)| (i64::from(c.0 - first.0)).pow(2) + (i64::from(c.1 - first.1)).pow(2)).map_or(0, |(i, _)| i);
    let (a, b) = ring.split_at(far);
    let mut first_half = a.to_vec();
    first_half.push(b.first().copied().unwrap_or(first));
    let mut second_half = b.to_vec();
    second_half.push(first);
    let mut out = rdp(&first_half, tolerance);
    out.pop();
    let mut tail = rdp(&second_half, tolerance);
    tail.pop();
    out.extend(tail);
    out
}

fn rdp(points: &[Corner], tolerance: f64) -> Vec<Corner> {
    let mut keep = vec![false; points.len()];
    if let Some(k) = keep.first_mut() {
        *k = true;
    }
    if let Some(k) = keep.last_mut() {
        *k = true;
    }
    let mut stack = vec![(0_usize, points.len().saturating_sub(1))];
    while let Some((lo, hi)) = stack.pop() {
        let (Some(&p), Some(&q)) = (points.get(lo), points.get(hi)) else { continue };
        let mut worst = (0.0_f64, None::<usize>);
        for i in lo.saturating_add(1)..hi {
            if let Some(&c) = points.get(i) {
                let d = distance_to_segment(c, p, q);
                if d > worst.0 {
                    worst = (d, Some(i));
                }
            }
        }
        if let (d, Some(i)) = worst
            && d > tolerance
        {
            if let Some(k) = keep.get_mut(i) {
                *k = true;
            }
            stack.push((lo, i));
            stack.push((i, hi));
        }
    }
    points.iter().zip(keep).filter_map(|(&p, k)| k.then_some(p)).collect()
}

fn distance_to_segment(c: Corner, p: Corner, q: Corner) -> f64 {
    let (cx, cy) = (f64::from(c.0), f64::from(c.1));
    let (px, py) = (f64::from(p.0), f64::from(p.1));
    let (dx, dy) = (f64::from(q.0) - px, f64::from(q.1) - py);
    let len2 = dx * dx + dy * dy;
    let t = if len2 > 0.0 { (((cx - px) * dx + (cy - py) * dy) / len2).clamp(0.0, 1.0) } else { 0.0 };
    let (nx, ny) = (px + t * dx, py + t * dy);
    (cx - nx).hypot(cy - ny)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid(rows: &[&str]) -> (Vec<bool>, usize, usize) {
        let h = rows.len();
        let w = rows.first().map_or(0, |r| r.len());
        (rows.iter().flat_map(|r| r.chars().map(|c| c == '#')).collect(), w, h)
    }

    #[test]
    fn a_rectangle_has_a_four_corner_contour() {
        let (m, w, h) = grid(&["....", ".##.", ".##.", "...."]);
        let c = largest_contour(&m, w, h).unwrap();
        let s = simplify(&c, 0.5);
        assert_eq!(s.len(), 4);
        assert_eq!(signed_area(&c).abs(), 4);
    }

    #[test]
    fn the_largest_region_wins() {
        let (m, w, h) = grid(&["#...###", "....###", "....###"]);
        let c = largest_contour(&m, w, h).unwrap();
        assert_eq!(signed_area(&c).abs(), 9);
    }

    #[test]
    fn empty_grid_has_no_contour() {
        let (m, w, h) = grid(&["...", "..."]);
        assert!(largest_contour(&m, w, h).is_none());
        assert!(largest_contour(&[], 0, 0).is_none());
    }

    #[test]
    fn diagonal_pinch_terminates() {
        let (m, w, h) = grid(&["#.", ".#"]);
        let c = largest_contour(&m, w, h).unwrap();
        assert_eq!(signed_area(&c).abs(), 1);
    }

    #[test]
    fn an_l_shape_keeps_its_corner() {
        let (m, w, h) = grid(&["##..", "##..", "####", "####"]);
        let c = largest_contour(&m, w, h).unwrap();
        let s = simplify(&c, 0.5);
        assert_eq!(s.len(), 6);
    }
}
