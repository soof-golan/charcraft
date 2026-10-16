# Neck weld: what the render must show

Written before the implementation. A welded head (Character Animator's default attach style) stays rigid; the body
mesh is what bends. The weld acts where the head's artwork overlaps the body's artwork, which for Boy is the top of
the neck under the chin.

1. Neutral pose: the render is pixel-identical to the artwork render.
2. Head moved 30 px up (no rotation): the face pixels (a crop of the eyes and mouth) equal the neutral crop shifted
   by 30 px, within a small tolerance. The head is not deformed, the chin and mouth never get pulled toward the shirt.
3. The neck is a continuous strip between the head and the collar: no transparent (background) pixel on the neck
   line between the chin and the collar, for the moved head.
4. The collar and the logo move less than 3 px when the head moves 10 px (the far body stays put; only the neck
   stretches).
5. The mouth never overlaps the shirt: the shirt colour never appears inside the mouth crop, and the mouth crop
   keeps its neutral colours (shifted with the head).

6. (Replaces the earlier sway-based point.) The body never moves with the head, as in Character Animator.
   a. The head rotates about its Neck handle (the Head group's origin tagged `Adobe.Walk.Neck` in the project, or a
      handle with that tag; the bottom of the bounds only when there is none). For a 15 degree tilt the chin's centre
      lands within 1 px of the point predicted by a rotation about that Neck point.
   b. The neck strip stays centred under the chin at its top and on the collar at its bottom, and bends as a smooth
      curve, not a sheared parallelogram: its centre line has no jumps between rows and its width, measured
      perpendicular to the centre line, stays at least 80% of rest along its whole length, for tilts up to 20 degrees
      and shifts up to 20 px either way.
   c. The body does not move: the collar line stays within 1 degree of horizontal, and shirt pixels outside a band
      of 1.5 neck widths around the neck differ from the neutral render by at most 6 levels (measured: a few
      pixels differ by up to 4 levels because the body is resampled through its mesh; the first prediction said 2).
      The neck's bottom may follow the head by up to half its sideways shift (measured 8 px for 20 px): the bend
      needs room, and the band around the neck is where the body is allowed to give.
   d. The face stays rigid and the neutral pose stays pixel-identical.
   Mechanism: the body mesh is refined so the neck band between the chin and the collar has several vertex rows;
   the overlap row is pinned to the head's pose, the collar row and everything below it to rest, and ARAP bends
   what lies between. A long neck for a big shift is acceptable; a skewed body is not.
7. Head dropped 20 px: the chin may overlap the collar (as in Character Animator), but no neck-coloured pixel
   shows above the chin, i.e. the neck never pokes out over the face.

Synthetic version (neck_tree in synthetic.rs, head moved 30 px up and rotated 15 degrees): every body-mesh vertex
inside the head's silhouette lands within 1 px of the head's matrix applied to its rest position; the far torso
vertices stay within 1 px of rest; no background pixels along the neck line.
