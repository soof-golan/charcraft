# punch-audio-alignment

Recorder with the microphone and the Head group armed and a punch range of 1.0..2.0 s. A 100 Hz ramp-coded signal
(sample value = sample index / 8000, mono, 8 kHz) is delivered in 0.25 s chunks starting at 0.125 s (so chunks
straddle the punch edges: 0.875..1.125 and 1.875..2.125), plus a late chunk with a 0.05 s hole in front of it.

Failure mode: audio outside the punch is recorded, samples inside the punch are lost with a dropped straddling
chunk, or a gap shifts later sound earlier against the picture.

## Predictions

1. The recorded audio starts at scene time exactly 1.0 s and ends at exactly 2.0 s (8000 frames at 8 kHz): audio
   that straddles a punch edge is cut at the edge, not dropped whole and not kept whole.
2. The value of recorded sample i equals the original signal at scene time 1.0 s + i/8000, i.e. `(8000 + i)/8000`
   (the ramp code makes misalignment visible): the first sample is 1.0, the last is 1.99987.
3. A hole in the stream (next chunk starts 0.05 s after the previous one ended) is filled with 400 zero samples,
   so every later sample still sits at its true scene time; an overlapping chunk (starts 0.02 s before the previous
   end) does not repeat the overlapped samples.
4. Audio chunks arriving before the recording started are dropped; a chunk straddling the start is cut at it.
5. Mixing the recorded clip back at `audio.start` reproduces the original signal at the same output samples (within
   1e-6) at 8 kHz.

## Verdicts
FAILED before the fix: the straddling chunk at 0.875 s was dropped whole, so the audio started at 1.125 s (0.125 s
of punched sound lost), and the straddling chunk at the punch-out was kept whole (sound beyond 2.0 s); holes
shifted later sound earlier. Fixed in `Recorder::push_audio` (cut at the punch and at the start, pad holes,
skip overlaps). All five points now hold.
