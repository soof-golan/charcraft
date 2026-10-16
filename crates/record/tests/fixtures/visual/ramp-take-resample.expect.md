# ramp-take-resample

A Head take recorded from a script at 30000/1001 fps (head roll = frame index in degrees up to 30, a plateau at 30
from frame 30 to 70 inclusive, then down by 1 degree per frame to 0 at frame 100; one `Face` event per frame) and
placed at scene time 0.5 s, then played in a scene of 24000/1001 fps (different rate, different phase).

Failure mode: interpolation between samples uses the wrong time base; the run-compression of the plateau eats the
ramp; take edges are off by one frame; the sidecar changes the numbers.

## Predictions

1. The take holds 62 samples: frames 0..29 (30), plateau first (frame 30) and last (frame 70), then frames 71..100
   (30). Take start = 0.5 s exactly, duration = tick_of(100) + 1 tick.
2. For each scene frame f with time t inside the take: `track_inputs(...).face.head_roll` equals the piecewise-linear
   curve through (tick_of_29.97(k), r(k)): s = (t - 0.5 s) in seconds; up: `s * 29.97003` for s <= 30/29.97; hold 30
   until s = 70/29.97; down: `30 - (s*29.97003 - 70)` until s = 100/29.97, within 1e-6. Other face fields stay at
   their neutral values (only the Head group was recorded).
3. Frames before 0.5 s (frames 0..11 at 23.976) and after the take end (> 0.5 + 3.33667 = 3.83667 s, from frame 92)
   give `face == None`.
4. At the plateau the settled pose rotation (frame 50..60 of the scene) is constant to 1e-3; mid-ramp (frame 22) it is
   smaller than at the plateau, and the rendered head differs (eyebrow pixel at its rest position changes).
5. The scene saved and loaded through the sidecar gives bit-identical `track_inputs`.

## Verdicts
All points confirmed (62 samples; 80 covered frames, 12..=91; curve within 1e-6 at every frame; sidecar identical).
Sheet `ramp-take-resample.png` (frames 0, 22, 66, 95): upright, slightly tilted, strongly tilted, upright again.
My first draft asked for byte-identical rest and end frames; that was too strong (the random blinker may be mid
blink), so the test compares the pose rotation and the eyebrow pixel instead.
