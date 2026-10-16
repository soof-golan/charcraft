# frame-tick-maths

No picture; the clock the whole crate hangs on. Scenes at 24000/1001, 24/1, 30000/1001, 30/1 and the awkward
11/1 and 60000/1001 (10 s long unless noted). `Tick` is 254 016 000 000 per second.

Failure mode: `frame_time` / `frame_at` / `frame_count` / `Script` times / take weights / mixer start sample disagree
by one frame at a boundary, or drift over long scenes.

## Predictions

1. `frame_time(f) = floor(f * 254016000000 * den / num)`: for 24000/1001 frame 1 is 10 594 584 000 ticks, for
   30000/1001 frame 1 is 8 475 667 200, for 24/1 frame 1 is 10 584 000 000, for 30/1 frame 1 is 8 467 200 000, for
   60000/1001 frame 1 is 4 237 833 600 ticks (each exactly, all divide evenly).
2. For all f in 0..=1_000_000 (sampled): `frame_at(frame_time(f)) == f` and `frame_at(frame_time(f) - 1) == f - 1`
   (the frame boundary is exclusive from below), at every rate above including 11/1, where the tick does not divide
   evenly.
3. `frame_count` of a 10 s scene: 240 at 24000/1001 (239.76 frames, last one partial), 240 at 24, 300 at 29.97
   (299.7), 300 at 30, 110 at 11/1, 600 at 59.94 (599.4). A scene lasting exactly `frame_time(N)` has N frames; one
   tick longer has N+1.
4. `Script::inputs` times are identical to `Scene::frame_time` for the matching frame (same rate, `fps_f64` fed
   through `FrameRate::from_f64`): for every common rate and for f in 0..3000.
5. A take that starts at `frame_time(k)` covers frame k (weight 1) and not frame k-1 (weight 0); a take that ends
   at `frame_time(k)` does not cover frame k.
6. Mixer: a clip placed at `frame_time(k)` begins at output sample `floor(k*1001/24 ms * rate)` for 48 kHz and 44.1 kHz.

## Verdicts
Point 2 FAILED at 11/1: `frame_at(frame_time(997))` was 996. Root cause: `Scene::frame_time` floored the frame
start, so for rates whose frame start is not a whole tick the tick lies in the previous frame (a take starting at
that tick would then also start one frame early, a clip start one sample early). Fixed: `frame_time` is now the
first tick inside the frame (ceil). The prediction text said "floor" in point 1 for the evenly dividing rates,
where floor and ceil agree; for 11/1 the correct value is the ceil. Points 1, 3-6 held from the start.
