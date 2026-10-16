# eye-gaze-limits

Puppet (240x240, built in `tests/visual.rs::face_tree(wide_lids = true)`): Head (Face rect 40..200 x 20..140, skin
`240,190,160`) with a Left Eye (white 130..170 x 60..84, pupil `+ Left Pupil` 12x12 at 144,66, black) and a Right Eye
(white 50..90 x 60..84, pupil at 64,66). Each eye group also holds a hidden closed-lid layer (`Left Blink`,
`Right Blink`) that is WIDER than the eye white: 56x4 at 122,70 and 42,70 (lashes poking out 8 px each side).
Head origin = bottom centre (120,140).

Failure mode aimed at: the pupil's travel range is computed from the eye group bounds, which include the hidden
blink layer; so the pupil leaves the eye white.

Rule: a pupil moves within the visible eye. Range per axis = (white size - pupil size) / 2 = (14, 6) px.
Gaze (gx, gy) is clamped per axis to [-1,1]; non-finite gaze means 0.

## Predictions (pupil boxes, x0..x1 / y0..y1)

| pose | left pupil | right pupil |
|---|---|---|
| A gaze (1,0) | 158..170 / 66..78 | 78..90 / 66..78 |
| B gaze (-1,1) | 130..142 / 72..84 | 50..62 / 72..84 |
| C gaze (5,-5) (clamped to (1,-1)) | 158..170 / 60..72 | 78..90 / 60..72 |
| D gaze (1,0), `pupil_left` = (-1,0) | 130..142 / 66..78 | 78..90 / 66..78 |
| E gaze (1,0), head turned 90 deg clockwise about (120,140) | see below | see below |
| F gaze (NaN, inf) | 144..156 / 66..78 | 64..76 / 66..78 |

Pixel probes (A): (164,72) black; (157,72) white; (169,72) black; (171,72) skin (NOT black: the pupil must not stick
out of the white). The right edge of the white is x=170, so no black pixel may exist at x >= 170 in rows 60..84
(left eye) in any pose.

E: a point (x,y) maps to (260 - y, x + 20). Left pupil box becomes 182..194 / 178..190, right pupil 182..194 /
98..110; probes (188,184) black and (188,104) black; (188,191) skin.

Invariant over all poses: no black pixel outside the union of the two white rectangles (mapped through the head
turn), i.e. every black pixel lies on the eye white or is covered by pupil within it.

## Verdicts

Rendered to `/mnt/data/anim/scratch/visual/charcraft-puppet-behaviors/eye-gaze-limits.png` (sheet A..F) and an 8x
crop of the left eye in pose A.

- BUG FOUND (first render): pose A left pupil was at 166..178, sticking 8 px out of the white (right edge 170);
  the 8x crop showed a black block hanging over the skin. Root cause: `Puppet::pupil_offset` took the eye group's
  bounds, which include the hidden, wider closed-lid layer, so the range was 22 px instead of 14. Fixed in
  `crates/puppet/src/eval.rs` (`shown_bounds` ignores hidden layers and blink layers).
- After the fix: A, B, C, D, F pupil boxes all exactly as predicted; (164,72) black, (157,72) white, (169,72)
  black, (171,72) skin: CONFIRMED. C (5,-5) equals the clamped (1,-1): CONFIRMED. D pupil override moves only the
  left pupil: CONFIRMED. F (NaN, inf) centred: CONFIRMED. E (head turned): pupils at the mapped boxes, nothing
  sticks out of the turned whites: CONFIRMED. No dark pixel outside the whites in any pose: CONFIRMED.
