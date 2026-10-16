# speech-to-mouth

Synthetic speech (16 kHz, glottal pulse train at 120 Hz through formant cascades, rms 0.1): 0.30 s silence, Ah
(800/1300/2600 Hz) 0.50 s, Ee (300/2250/3000) 0.50 s, 0.40 s silence, Oh (540/880/2500) 0.50 s, 0.30 s silence.
Boundaries: Ah 0.30..0.80, Ee 0.80..1.30, silence 1.30..1.70, Oh 1.70..2.20, silence 2.20..2.50.

The mouth track from `anim_lipsync::analyze` is sampled at every frame of the scene (`viseme_at(frame time)`), put into
a Voice take through the recorder, played through the Lip Sync behavior and drawn on a 200x200 puppet whose 14 mouth
shapes are flat rectangles of distinct colours at puppet (85..115, 90..105) (scene = puppet pixels). Run at 23.976,
24, 29.97 and 30 fps.

Failure mode: the recorded take, the frame clock (`Tick` floor maths) and the Lip Sync hold logic disagree by a frame,
swallow a one-frame viseme, or shift speech against the picture at NTSC rates.

## Predictions

1. Mouth centre pixel (100,97) is the Neutral colour at every frame whose time is < 0.28 s, in 1.45..1.65 s and
   > 2.3 s.
2. It is the Ah colour at every frame in 0.42..0.68 s, the Ee colour in 0.92..1.18 s and the Oh colour in 1.82..2.08 s.
3. The recorded take holds exactly the sampled sequence: replaying `viseme_at(keys, t)` at each frame time gives the
   same pose viseme (puppet-side mapping) at the same frame, at all four rates (no off-by-one frame).
4. The first frame that shows a viseme is the first frame whose time is >= the key's start (a key is active at its
   own start tick). The frame before shows the previous viseme.
5. The pixel at (100,97) is fully opaque and equals the shape colour exactly (flat rectangle, no blending); the
   rectangle edges are at x=85 / x=114 and y=90 / y=104 (inclusive), 1 px outside is skin (240,190,160).
6. Rendering a frame twice from two separately built `SceneFrames` is byte-identical.

## Verdicts
All six points confirmed at 23.976, 24, 29.97 and 30 fps (every frame compared, not just the zones). Sheet
`speech-to-mouth.png` (0.5, 1.0, 1.5, 2.0 s at 23.976 on top, 29.97 below): Ah, Ee, Neutral, Oh. Crop
`speech-to-mouth-edge.png` (8x): rectangle edges at x 85..=114, y 90..=104 with no blending. The analyzer
produced exactly Neutral, Ah, Ee, Neutral, Oh, Neutral.
