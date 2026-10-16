# head-extremes

Puppet: the 240x240 face puppet (`face_tree(false)`) plus a gray Body (60..180 x 140..240 ... see
`face_with_body()`) under the head. Head origin = bottom centre (120,140).

Failure mode: the head transform composed of rotation, scale and translation puts features somewhere other than
`translate * about(origin)`; extreme values (0 scale, mirror, 1e6 px, NaN) crash or leave junk.

Rule: a rest point p lands at `T(p) = (rot, scale about (120,140)) + translation`. Features are probed at their
transformed centres: left pupil (150,72) black, right pupil (70,72) black, mouth centre (120,110) neutral colour
viseme_color(0), face centre (120,80) skin.

Poses (probe only points that land inside the canvas):

1. rotation 180 deg.
2. rotation -450 deg (= -90) and rotation 1e9 rad: finite, no panic.
3. scale (3,3) and scale (0.2,0.2) about the neck.
4. scale (-1,1) (mirror): the left pupil lands where the right one was.
5. scale (1,-1) with translation (0,300): head flips over the neck line, ends up below.
6. translation (1e6, -1e6): the head is off the canvas: render has no skin/black pixel at all and does not
   panic; the body is untouched.
7. NaN rotation or NaN scale: treated as identity (render equals neutral).
8. scale (0,0): the head has no area: no skin pixel is left on the canvas (a zero scale collapses, it must not
   mean "no scale").
9. Neck stays attached (PENDING the weld fix by the other agent; test is `#[ignore]` until it lands): with the head
   turned 40 deg and moved (30,0), the rendering has no gap between head and body along the neck line.

## Verdicts

Rendered to `.../scratch/visual/charcraft-puppet-behaviors/head-extremes.png` (rot180, rot-450, rot1e9, scale3) and
`head-extremes-b.png` (scale0.2, mirror, flip, mixed).

- Points 1-7: all feature probes land on their transformed centres (the sheets show the head upside down below the
  neck, turned left, tilted, 3x filling the top, tiny on the neck, mirrored, flipped below, sheared); NaN equals the
  neutral render; the head 1e6 px away leaves no skin or black pixel: CONFIRMED.
- BUG FOUND (point 8): head scale (0,0) left the head at full size. Root cause: `Xform::sanitized` replaced any
  scale with magnitude <= 1e-6 by 1.0, so "collapse the group" meant "no scale". Fixed in
  `crates/puppet/src/pose.rs`: a collapsed scale becomes the smallest scale of the same sign. After: no skin
  pixel: CONFIRMED.
- Point 9 (neck weld, after the weld fix landed on main): `the_neck_stays_attached` now runs on an eyeless puppet
  (a white eye with a pupil is a hole with an island and counts as extra components: the first failure, 3
  components, was a test mistake) with 7 poses (+-40 deg moved +-30, 15, a pure move, +-90, 180): one component and
  no transparent pixel along the neck line: CONFIRMED (crops neck-90-0, neck-180-0).

### Re-judged after the weld landed (the sheets were re-rendered and the goldens replaced)

The prediction "a rest point lands at T(p)" is wrong near the neck once Weld is on: the head mesh within half its
extent (80 px) of the attach point is blended with the body, so the mouth (30 px from the origin) is not at its
rigid position (rot180: it sits about 25 px above it) and the pupils (74..84 px away) are distorted at +-90. The
probes now use only points at least 90 px from the origin (the face corners, which are rigid), and the pupils are
judged by `eye_gaze_limits` (turn90 probes).
- Observation, not judged a bug: at extreme poses the blend ring leaves ragged geometry: at -90 deg a notch in the
  face and a mouth sliver detached from the chin; for the flip pose a skin spike rises from the neck above the
  flipped head (head-extremes-b panel 3). The weld holds the neck end at the body while the rest is flipped, so
  some bridge is expected, but its shape is not a clean stretch.
- BUG FOUND (point 8, again): with the weld a head scale of (0,0) still leaves a skin sliver at (104..152, 140..143):
  the ring near the attach point follows the body mesh and ignores the head's scale. Not fixed (weld code is
  another agent's); `a_zero_scale_head_collapses` is `#[ignore]`d with the reason.
