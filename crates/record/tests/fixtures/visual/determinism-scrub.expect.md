# determinism-scrub

Scene 29.97 fps, 6 s, seed 7, two tracks of the same puppet ("A" with a Head take and a Voice take with trims set on
the Lip Sync behavior via a track edit; "B" with nothing). Blinks are random (seeded), the Lip Sync trims and the
smoothers are stateful.

Failure mode: a pose depends on how the evaluator was driven (straight, backward, jumping across the 60 frame
snapshots) or on another track; two renders of one frame differ by a bit.

## Predictions

1. Frame 100 rendered from `SceneFrames` built twice is byte-identical, and `display_list(100)` queried before and
   after rendering other frames is equal.
2. `Evaluator::pose(0, f)` after jumping in the order 179, 61, 59, 60, 120, 121, 0, 119, 180 equals the straight
   `bake` poses at those frames (snapshots at multiples of 60: frames 59/60/61/119/120/121 sit on both sides of each).
3. Track A's poses are bit-identical whether or not track B exists or is disabled (tracks are independent), and a
   track's blink pattern depends on the scene seed + track index only.
4. Two tracks with the same puppet and same seed index give identical blink times only when their indices (hence seeds)
   are equal: changing the scene seed changes the blink frames; the same seed reproduces them.
5. Lip Sync Trim_Ah of 200 ms keeps the Ah colour on screen for 200 ms (6 frames at 29.97) after the voice leaves it
   for Oh, and a trim never delays the very first shape.

## Verdicts
All points confirmed. Point 5 precisely: Ah was last wanted at frame 59, so with Trim_Ah 200 ms it stays through
frame 64 and Oh shows from frame 65 (5 frames = 166.9 ms are short of 200 ms, 6 frames = 200.2 ms after frame 59);
my first wording of "6 frames after the voice leaves" was off by one, the rule counts from the last wanted frame.
