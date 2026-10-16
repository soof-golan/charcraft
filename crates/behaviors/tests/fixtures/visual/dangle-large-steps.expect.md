# dangle-large-steps

Puppet (240x300, `hair_tree()`): Body (gray, 80..160 x 150..250); `Head` (skin 60..180 x 20..120, origin = bottom
centre (120,120)) containing an independent group `+ Hair`: a purple (130,40,160) strand 112..128 x 100..170 hanging
past the chin, with handles `Fixed` (120,105) and `Dangle` (120,165). The strand is a child of the head, so it
rides the head turn. Hair origin = strand centre (120,135); the dangle reach is therefore 30 px.
Stack: Face (smoothingRate 0, headRotFactor 100, so head roll in degrees is the head rotation) then Dynamics
(defaults).

Failure mode: big time steps (a whole second in one tick) and a sudden 90 degree head swing make the integrator
explode, the tip fly out of reach, tear the mesh or leave the strand stuck displaced after it should have settled.

## Predictions

1. Scenario S (30 fps): 2 s neutral, then roll = 90 degrees held for 8 s. Every pose is finite; at every frame
   the stage-space distance between the dangling tip and its driven anchor is <= reach (30 px) + 0.5.
2. Scenario L (one big step): 1 tick at t=0 neutral, one tick at t=1.0 s with roll 90 (dt = 1 s), then 30 fps for
   8 s. Same bound at every frame, including the big-step frame. The strand is never torn: the drawing of
   head+hair is one connected component.
3. After settling (frame at 8 s after the swing) the drag is below 0.5 px in both scenarios, and the rendered
   images of S and L at that frame match to within 2 levels per channel on at least 99.5 % of pixels.
4. Head turned 90 degrees clockwise about (120,120): (x,y) maps to (240-y, x). The settled strand (rest rect
   x 112..128, y 100..170) maps to x 70..140, y 112..128 (face maps to x 120..220, y 60..180). Probes: (130,120) and
   (100,120) purple (hair is above the face and attached at the chin end), (74,120) purple (near the tip),
   (66,120) white background (beyond the tip), (100,110) and (100,131) white (strand is 16 px thick).

## Verdicts

(filled in after rendering)
