# blink-viseme-swap

Puppet (240x240, `face_tree(wide_lids = false)`): same head as eye-gaze-limits but the hidden closed lids are
`40x6` blue (30,60,220) bands at 130,69 (left) and 50,69 (right), inside the eye groups, drawn last (above white and
pupil). Mouth group `mouth` at 100..140 x 100..120 with 14 shapes, each a distinct colour `viseme_color(i)`; Neutral
is visible by default.

Failure mode: blink and viseme switches interact wrongly when they happen in the same frame, under a rotated head,
at the thresholds (0.49 / 0.5), with NaN, or with visibility overrides.

Rules: a blink layer shows from closed amount >= 0.5 (finite); showing it hides every sibling in the eye group
(white AND the independent pupil group). Left uses `blink_left`, right `blink_right`. The requested viseme's layer
is the only mouth layer shown; the neutral one is the fallback. Head turn 90 deg cw maps (x,y) to (260-y, x+20).

## Frames

1. Ah, no blink: mouth centre (120,110) = viseme_color(1); (150,72) black (pupil); (150,64) white.
2. Oh, blink_left 0.49, head 90 deg: eye still open: (188,150) ... mapped from (150,72) is (188,170) black; mouth
   (120,110) -> (150,140) = viseme_color(7).
3. Oh, blink_left 0.5 exactly, no head turn: (150,72) blue (band y 69..75); (150,64) skin (white hidden);
   (150,78) skin; (150,67) skin (pupil hidden); right eye unaffected: (70,72) black.
4. Ee, both blink 1.0, head 90 deg: bands at left (188,170) blue and right (188,90) blue; mouth (150,140) =
   viseme_color(3); no black pixel anywhere in the eyes' region.
5. Surprised, blink_left NaN, blink_right 2.0: left eye open (150,72) black; right eye shut (70,72) blue;
   mouth = viseme_color(13).
6. Neutral viseme not set, blink 0, visibility override `Left Blink` = true: the lid is forced on but nothing is
   hidden: (150,72) blue (band over the pupil, lid is the top layer), (150,67) black (pupil above the band),
   (135,62) white.

Extra assertion (puppet without a Neutral shape; only Ah (visible), Oh, Ee): asking for Surprised leaves the
artwork's own visible shape (Ah) showing instead of an empty mouth.

## Verdicts

Rendered to `.../scratch/visual/charcraft-puppet-behaviors/blink-viseme-swap.png` (sheet of frames 1..6) with 8x
crops of the left eye for frames 3 and 6.

- Frames 1-6 all as predicted. 0.49 leaves the eye open, exactly 0.5 closes it (blue band y 69..75, white, pupil
  and the area under the band all gone: crop of frame 3 is skin with one blue bar). NaN does not blink, 2.0 does.
  Under a 90 degree head turn the lids and mouth land at the mapped positions. Forced lid (frame 6, crop): the
  band sits over the pupil with the pupil's top and bottom and the white still showing: CONFIRMED.
  (Frame 2's text above has a typo, the mapped pupil is at (188,170).)
- BUG FOUND: a mouth group without a Neutral shape, asked for a shape it also lacks (Surprised), came out EMPTY
  (pixel (120,110) was skin): `Puppet::visibility` hid every shape when no shape matched and no neutral existed.
  Fixed in `crates/puppet/src/eval.rs`: with no match the artwork's own shape stays. After: CONFIRMED (shows Ah);
  a shape that exists still wins: CONFIRMED.
