# Head parts: ears and hair on the anchors

Written before the fix; the real puppets are `male_anchor` and `female_anchor` in
`chproj_full/news anchors` (tests in `tests/puppet/samples.rs`, skipped without `ANICRAFT_SAMPLES`).

1. Male anchor, ears. The ears are one layer (`Layer 3`) that the PSD draws behind the face, as a sibling of the
   `head` group, not inside it, so the head turning left them behind. Character Animator moves the groups attached
   to a head and the ones that overlap it with the head. Prediction: with the head tilted 15 degrees (also -15 and
   25), the pixels along a segment from just inside the left ear's tip to the face, carried through the head's
   matrix, are all puppet pixels (no background between ear and face), and the layer is reported as following the
   head. The body and its arms do not follow (only `Layer 3` does): their overlap with the head's bounds is small.
   Rule: a sibling of the head, not tagged as a head, not an independent group set to the Free attach style, with at
   least 60% of its artwork inside the head's bounds follows the head about the head's pivot.
2. Female anchor, jacket. Hypothesis A: the jacket is torn by the weld pins (body vertices far from the neck pinned
   by the hair overlap). Hypothesis B: the hair (part of the head, correctly rotating) uncovers a ragged edge that
   the jacket artwork itself has under the hair. Test: hide the head (hair front included) and tilt it. Under A the
   body renders differently from the straight-head render far from the neck; under B the two renders agree and the
   ragged shoulder is there in both. Prediction: B. Outside 260 px of the neck the two renders differ by at most
   0.5% of the pixels (one-pixel edge resampling through the body mesh).
