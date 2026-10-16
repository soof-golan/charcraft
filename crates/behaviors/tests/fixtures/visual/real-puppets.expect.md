# real-puppets: predictions for head, drag, gaze and blink poses on the sample puppets

Written before the first render. Puppets (Adobe-authored, never committed; the test reads `ANICRAFT_SAMPLES` and is
skipped without it): `Boy.puppet`, `blood_cells.puppet` (a girl character), and the two PSD news anchors
(`male_anchor.psd`, `female_anchor.psd`). Each is driven by its own behavior stack (the PSDs: the standard stack) with
scripted face and pointer inputs, settled for 60 frames at 30 fps. Face strengths are the defaults: head position 100%,
rotation 75%, parallax 100%. Numbers below are for a 1080 px stage (Boy, girl); the anchors scale by their stage size.

Head rotation pivots about the head's own origin (the neck), clockwise on screen for a positive roll.

## Poses

1. **Head turn** (yaw 30 degrees, then -30): there are no turn layers, so the face behavior only slides the head by
   the parallax amount, 30/45 x 1080 x 0.02 = 14.4 px toward the turn (yaw is positive toward the image's right).
   Predicted: the whole head, face features included, shifts about 14 px, rigidly, no rotation; the neck stays one
   continuous strip into the collar; the silhouette keeps its component count; with pitch 20 the head also rises 9.6 px.
2. **Head tilt** (roll +20, then -20): rotation of 15 degrees (75%) about the neck pivot. Predicted: the top of the
   head moves toward the roll direction (clockwise for +), the chin swings the other way by a few pixels; the face
   stays rigid (eye line rotated by 15 degrees); the neck remains continuous between chin and collar, bending, not
   tearing; the collar and shirt do not move by more than 3 px.
3. **Head moved** (translation x +0.6 of the camera frame, then up -0.6, then down +0.6): 0.6 x 1080 x 0.5 x 0.3 =
   97 px. Predicted: the head translates rigidly by 97 px; the neck stretches to follow without a gap (no transparent
   pixel on the line between chin and collar for up and sideways moves); for the drop the chin may overlap the collar but
   no neck-coloured pixel shows above the chin; the body far from the neck stays within 3 px.
4. **Move plus tilt** (translation 0.4, roll 20): both effects compose; the weld holds (one component, neck strip
   continuous). This is the case the neck-weld expectation asks for with rotation on every puppet.
5. **Draggers** (Boy: each draggable hand pulled 200 px up and out; the girl: her draggable handles): the pulled
   handle ends within a few pixels of the pointer (the handle follows the pointer within its parent's group
   space), the arm stretches along the pull without tearing (silhouette stays one component), and everything that is
   not the pulled group stays put (head and the other hand within 3 px).
6. **Eye gaze extremes** (gaze (1,0), (-1,0), (0,1), (0,-1), (1,1)): each pupil stays inside its own eye white and
   moves toward the gaze direction by a visible amount; both eyes move the same way; with the head rotated 15 degrees
   the pupils move with the head (they stay inside the whites). The anchors have no eyes: the render must be identical
   to the same pose without gaze.
7. **Blink** (eye_open 0 on both eyes, then on one eye): the eyelid or blink layer shows and the open pupil
   disappears from view on the closed eye; the other eye stays open. The anchors have no eyes: identical to neutral.

## Pass conditions asserted by the test (after the renders are judged)

- Neutral render equals the pose render with no input (idempotent).
- For poses 1 to 4 on every puppet: the number of connected opaque regions equals the neutral one (the head never
  detaches), and the transparent pixel count on the neck line stays zero.
- Pose 5: the pulled handle's position is within 10 px of the pointer.
- Pose 6 and 7 on the anchors: pixel-identical to neutral.
