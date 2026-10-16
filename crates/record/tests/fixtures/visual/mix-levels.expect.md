# mix-levels

Constant-value media at 8 kHz so levels can be read off directly; output `MixFormat` 8 kHz stereo unless noted.
Scene 24000/1001 fps, 2 s.

- Track 1 (gain 0.5): clip A = mono 0.5, start 0, pan -1, gain 1; clip B = stereo (L 0.2, R 0.6), start 0.5 s,
  gain 0.5, pan 0, trim_in 0.25 s, duration 0.5 s (so it sounds 0.5..1.0 s and its media is 1.0 s long).
- Track 2 (gain 1): clip C = mono 0.9, at frame 24 (frame_time(24) = 1.001 s), gain 2.0, pan +1, 0.5 s long (media).
- Track 3 (disabled): a loud clip. Clip D (disabled clip, track 2). Clip E names a missing medium.

Mono sources use a constant-power law with +3 dB at the hard sides (gain sqrt(2) at pan -1/+1, 1.0 per side at centre);
stereo uses a balance; track and clip gains multiply; the sum is not clipped by the mixer but is clamped to -1..1
in the video soundtrack.

## Predictions

1. `missing == ["ghost"]` and the mix is 2 s = 16000 frames.
2. A alone (0.0..0.5 s): L = 0.5 * sqrt2 * 0.5 (track gain) = 0.353553, R = 0.
3. B (0.5..1.0 s): L = 0.2 * 0.5 * 0.5 = 0.05, R = 0.6 * 0.5 * 0.5 = 0.15 (centre balance leaves stereo unchanged).
4. Nothing between 1.0 s and 1.001 s (frame_time(24) = 1.001 s, sample 8008): samples 8000..8007 are 0.
5. C (from sample 8008 for 4000 frames): R = 0.9 * 2.0 * sqrt2 = 2.545584, L = 0; the soundtrack clamps R to 1.0.
6. After sample 12008 everything is 0; disabled tracks/clips and the missing medium contribute nothing.
7. Mono output format averages: the same scene mixed to 1 channel at B gives (0.05 + 0.15)/2 = 0.1.
8. 48 kHz output: C starts at output sample floor(1.001 * 48000) = 48048 and its first sample is non-zero while 48047
   is zero.

## Verdicts
All eight points confirmed numerically (no picture). Note the comment in anim-audio's mixer says centre pan is -3 dB
per side, but the code (and the prediction) gives 1.0 per side at centre and sqrt2 at the hard sides: constant
power with unity centre. Behaviour is consistent; only the comment is misleading.
