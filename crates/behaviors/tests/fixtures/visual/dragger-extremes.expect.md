# dragger-extremes

Puppet (400x300, `torso_tree()`): group `Body` = torso 80..160 x 100..200 (gray 120,120,130) with a red belt
(200,40,40) at 80..160 x 150..160. Handles: `Fixed` at (120,190), `Draggable` at (120,110). Key `Body/Draggable`.
Behavior stack: Dragger only (defaults: grab radius 40, release 0.2 s, return on mouse up), 30 fps.
Rig reach of the handles = size of the group = 100 px.

Failure mode: a draggable pulled far beyond its reach (hundreds of px, off the canvas, negative infinity-ish) tears
or collapses the mesh, or the Fixed base moves, or a release does not return exactly to rest.

## Predictions

1. Pointer down at (120,110) (exactly on the handle) with no motion: pose identical to neutral (render equal to
   the neutral render pixel for pixel).
2. Pointer dragged to (150,110), (270,110), (120,-300) and (-5000,110) while held (unlimited range): at every one
   of these frames
   - the Fixed base holds: pixel (120,195) is gray and (125,185) is gray or red-free torso colour (the belt is
     above the base) - concretely (120,195) = gray;
   - the drawing is ONE connected region (4-neighbour components of non-white pixels == 1) wherever it is visible
     on the canvas, and the canvas is not blank;
   - pose.drag["Body/Draggable"] equals pointer - handle rest exactly (delta (30,0), (150,0), (0,-410),
     (-5120,0)).
   For (270,110): the top of the torso follows the handle, so pixel (262,112) is torso gray.
3. `limit_range_cb = true`, `range = 20` (% of reach 100): the same pointer paths give |drag| <= 20 exactly:
   pointer (270,110) -> drag (20,0); (120,-300) -> (0,-20). Render has one connected component.
4. A pointer that goes down 70 px from the handle (120,40) grabs nothing: pose.drag stays empty for every frame,
   render equals neutral.
5. Release (on_mouse_up = return, duration 0.2 s): halfway through the glide (0.1 s after the release frame,
   smoothstep(0.5) = 0.5) the drag is half of the release displacement (+-1 px); 0.2 s + 1 frame later the drag is
   gone and the render is pixel-identical to neutral.
6. Two pointers at the same time: pointer 0 (down on the handle) holds it; pointer 1 down on the same handle
   does not steal it (handle belongs to pointer 0 until it is released).

## Verdicts

(filled in after rendering)
