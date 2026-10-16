# overlap-and-nested

Two puppets.

## overlap (240x240, `overlap_tree()`)

Independent groups (names with `+`), bottom to top: `+ Red` 40..100 square (200,30,30), `+ Blue` 70..130
(30,60,220), `+ Green` 100..160 (30,170,60). Origins = centres: red (70,70), blue (100,100), green (130,130).

Failure mode: draw order follows the order groups were moved, transforms leak between groups, scale 0 does not
collapse a group, pose keys hit the wrong group.

Predictions (white background elsewhere):

- Neutral: (50,50) red; (85,85) blue (blue above red); (95,95) blue (green starts at 100); (115,115) green;
  (65,95) red; (150,150) green.
- A. red moved by (+50,+50) (now 90..150): draw order stays red < blue < green: (95,95) blue; (110,110) green;
  (135,92) red (outside blue and green); (145,145) green; (55,55) white.
- B. green rotated 45 degrees about (130,130) (diamond with half-diagonal 42.4): (130,165) green; (128,90) green
  (inside the diamond, above blue); (101,101) is OUTSIDE the diamond (diagonal distance 41 but the diamond edge is
  30 along the diagonal) so it shows blue; (159,159) shows white.
- C. blue scale (0,0): the group collapses, so (85,85) shows red and (115,115) green; (110,75) white... blue
  area 70..130 x 70..130 outside red (40..100) and green (100..160) is white: (120,80) white.
- D. root moved (+10,0) and red hidden by `visibility["+ Red"] = false`: everything else shifted 10 px: (95,85)
  blue (was 85), (45,50) white.

## nested warps (300x240, `arm_tree()`)

`Body` (independent by tag) holds torso 80..160 x 60..180 (gray 120,120,130), handles `Fixed` (120,170) and
`Draggable` (120,70) (key `Body/Draggable`), and a child independent group `+ Arm` (blue 30,60,220, 150..220 x
100..120, origin = its centre (185,110)) with its own `Fixed` (155,110) and `Draggable` (215,110) (key
`+ Arm/Draggable`... label `Arm/Draggable`). Arm overlaps the torso by 10 px (150..160).

Failure mode: the child group's own origin lies outside the parent's mesh, so the parent's warp does not carry
the child, and the arm is left behind when the body bends away (gap at the shoulder).

- N1. Body draggable pulled by (-40,0): the torso leans left about its base. At the shoulder row y=110 the body
  right edge moves left by about 24 px (60/100 of 40), so the arm must follow: the picture is ONE connected
  component and the arm's leftmost blue pixel is at x in [120,140] (not at 150).
- N2. Arm draggable pulled by (0,-50), body at rest: the arm bends about its Fixed shoulder end; the shoulder end
  stays attached: pixel (152,110) is blue or gray, ONE connected component, and the arm tip rises: (215,64) is
  blue.
- N3. Both at once: still ONE component.

## Verdicts

(filled in after rendering)
