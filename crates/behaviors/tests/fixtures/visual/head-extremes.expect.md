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
- Point 9 (neck weld): test `the_neck_stays_attached` is `#[ignore]` until the weld fix lands on main (other
  agent); not judged here.
