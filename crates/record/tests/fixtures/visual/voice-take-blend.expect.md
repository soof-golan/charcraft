# voice-take-blend

Scene 10 fps, 3 s. Track "Hero": take 1 (Voice) holds Ah for the whole scene. Take 2 (Voice) holds Oh from 1.0 s for
1.0 s, blend in 0.4 s and blend out 0.4 s. Voice is a discrete group: the upper take wins from weight 0.5 up.

Failure mode: the blend switchover lands a frame early or late, the lower take does not come back, or blends of a
discrete group produce a third (mixed) shape.

## Predictions

Frame f is at t = f/10 s. Weight of take 2: ramps 0 -> 1 over 1.0..1.4, 1 until 1.6, 1 -> 0 over 1.6..2.0.
Mouth colour at (100,97):

1. Frames 0..11 (t < 1.2): Ah. Frame 11 weight 0.25.
2. Frames 12..18 (1.2 <= t <= 1.8): Oh. Frame 12 has weight exactly 0.5 (switches at >= 0.5); frame 18 (weight 0.5 on
   the way out) is still Oh.
3. Frames 19..29: Ah again (frame 19 weight 0.25, frame 20 is outside the half-open range).
4. Never any colour other than the Ah and Oh shape colours at the mouth centre.
5. Disabling take 2 gives Ah on all frames; removing the blend (0/0) makes Oh exactly frames 10..19 (half-open).
6. A face-group take with the same blends changes nothing about the mouth (independent groups).

## Verdicts
All points confirmed. Sheet `voice-take-blend.png` (frames 10, 11, 12, 17, 18, 19): Ah, Ah, Oh, Oh (with a seeded
blink), Oh, Ah. Frame 12 (weight exactly 0.5) switches, frame 18 stays Oh, frame 19 is back to Ah.
