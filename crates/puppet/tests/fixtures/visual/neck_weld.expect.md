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

Synthetic version (neck_tree in synthetic.rs, head moved 30 px up and rotated 15 degrees): every body-mesh vertex
inside the head's silhouette lands within 1 px of the head's matrix applied to its rest position; the far torso
vertices stay within 1 px of rest; no background pixels along the neck line.
