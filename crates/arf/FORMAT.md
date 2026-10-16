# The `.arf` event graph

Reverse engineered for interoperability from the 23 recordings of the `news anchors` sample project (16 live takes, 4
`Visemes_*` transcript recordings and 3 empty takes under `Ch Media/Recordings/`, plus the 6 compiled copies under
`Ch Data/temp.noindex/compiler/sources/`). Nothing here comes from Adobe code. `tests/samples.rs` reads every one of
them and writes it back **byte for byte**.

## What the files are

Every take Character Animator records is an event graph: for each recorded signal ("channel") a curve of its value
over time. The project only stores an alias (`.Recordings/Recordings/<date>/take 01_44_11 pm.arf`, class
`EventGraphFile_000`); the data is the file of the same path under `Ch Media/`. A scene's take
(`EventGraphPileItem_` in a behavior instance's `_egPile`) names the file in `_sourceEventGraphPPath`, the behavior
instance that plays it (`_boundInstanceID`, such as `TIB47`), and where it sits on the timeline (`_timeItem`).

| Folder | File | Channels |
|---|---|---|
| `Recordings/<date>/` | `take HH_MM_SS pm.arf` | the live take: face tracker values, or Dragger handle values, or the voice |
| `Recordings/Transcript/` | `Visemes_YYYY_MM_DD_HH_MM_SS.arf` | `Viseme/LookaheadIndex` only: the mouth shapes of the transcript, aligned to the voice |
| `Recordings/<date>/` (empty) | | no frame, no channel: a take armed and stopped at once (432 bytes) |

A recording whose three armed behaviors hold nothing (58576 bytes for 104 s) has 416 frames and no channel.

## Container

```
group  "< TAG >"  u64be length  children  "</TAG >"
data   "[ TAG ]"  u64be length  payload  0xAC padding to a multiple of 8  "[/TAG ]"
```

A tag is four ASCII characters. A group's `length` counts its children (not the 16-byte header or the 8-byte
trailer); a data chunk's counts the payload without padding. Lengths and counts are big-endian, doubles little-endian.
A `str` is a `u64be` byte count and the UTF-8 bytes.

## Tree

```
EGRF
  VERS  VMAJ u32 = 2   VMIN u32 = 0
  FrmS                                          the recording cut into frames
    FrmL                                        (repeated)
      FrmA  str start  str end                  seconds
      FrCL                                      the curves of this frame
        FCIL                                    (repeated, one per channel that has a curve here)
          FCIA  u32 channel                     index into ChnL
          FCTA  u8 kind = 1                     1 is a run of cubic Bezier segments
          CrvA  u64 n, then n segments of 8 f64le   t0 v0 t1 v1 t2 v2 t3 v3
  FtrL
    StTA str   EdTA str                         the recording's start and end
    FDTA  u64 = 1  u64 = 4                      constant in every sample
    RSpA  str = "1"                             constant in every sample
  FrPL  FrPA u64 index  u64 offset  ...         file offset of each FrmL, in order (a pure index)
  ChnL  (ChkA str name  ChkV u32 channel) ...   the channel table
```

- **Times are text**: a double printed with `%.36g`, the exact decimal expansion rounded to 36 significant digits with
  trailing zeros dropped (`323638.973301917023491114377975463867`, `0.25`, `0`). `decimal::format_time` reproduces
  all 14172 sample strings; reading and writing therefore need no raw copy of the text.
- **Frames** are windows of 0.25 s counted from `StTA` (live takes: the application clock, hundreds of thousands of
  seconds; transcript and empty takes: 0). The last frame ends at `EdTA`; when `EdTA` falls exactly on a window edge a
  final zero-length frame follows. Frames are not skipped when nothing was recorded in them.
- **FrPL** repeats where each `FrmL` starts, so a reader can seek; the writer recomputes it.
- **ChnL** is stored in hash order (not sorted by index or name); the writer keeps the order it read. `ChkV` numbers
  are what `FCIA` refers to.
- **Curve segments**: the four control points of one cubic Bezier in (time, value). A held value is a flat segment
  (the four values equal), a jump is a vertical one (the four times equal, the values jumping from the first pair to
  the second), a measured signal is a smooth run. A segment starts where the previous one ended, and the last end
  point is repeated as the next segment's first. A frame's curve holds only segments inside the frame (an empty
  curve, `n = 0`, is common). At a jump the value after it is the value at that time.
- A segment's time control points never run backwards, and segments are in time order across frames, in every sample
  (checked by the tests).

## Channels

| Channel (path) | Values |
|---|---|
| `Viseme/LookaheadIndex`, `Viseme/Index` | Lip Sync viseme number: 0 Neutral, 1 Ah, 2 D, 3 Ee, 4 F, 5 L, 6 M, 7 Oh, 8 R, 9 S, 10 Uh, 11 W-Oo (the order of `anim_lipsync::Viseme::ALL`). -2 before the first word. Held values with jumps. |
| `Viseme/InputEnabled` | 1 while the voice input is on |
| `Audio/Amplitude` | the voice's loudness, smooth, roughly 0 to 0.5 |
| `BehaviorOutput/Adobe.FaceTracker2.js/cameraInput/TIBn/Filtered/Head/Head/<name>` | the Face behavior's filtered tracker values, below |
| `BehaviorOutput/Adobe.EyeGaze.js/cameraInput/TIBn/Filtered/Head/Head/<name>` | the same twenty names after the Eye Gaze behavior |
| `BehaviorOutput/Adobe.Dragger3.js/Input/TIBn/HandleTarget/../B2/B8/B4:/Draggable/<value>` | a draggable handle; the group is the take's `_takeGroupID` |

`TIBn` is the behavior instance on the track item that recorded the channel. A handle's address is the chain of
binding ids from the puppet down to the layer that carries it (`B2/B8/B4:` is `Male_anchor/Body/Arm Right` in the
sample puppet), followed by the handle's tag.

### Face values

Twenty channels per recording, named after `Head/Head/`: `DX`, `DY`, `DZ`, `Orient/X`, `Orient/Y`, `Orient/Z`,
`LeftEyebrow`, `RightEyebrow`, `LeftEyelid`, `RightEyelid`, `MouthSX`, `MouthSY`, `MouthDX`, `MouthDY`, `Scale`,
`MouthShape`, `LeftEyeGazeX`, `LeftEyeGazeY`, `RightEyeGazeX`, `RightEyeGazeY`. All are relative to the calibrated rest
pose of the face, so a face at rest reads about 1 for the ratios and 0 for the offsets.

| Channel | Observed range (4 face takes) | Meaning |
|---|---|---|
| `Orient/X`, `/Y`, `/Z` | -0.6 to 0.6 | head turn in radians about x (nod), y (shake) and z (tilt) |
| `DX`, `DY` | -0.7 to 0.4 | head position in the camera frame, fraction of its size |
| `DZ` | -2.1 to 2.2 | head distance (unitless; mirrors `Scale`) |
| `Scale` | 0.8 to 1.3 | head size relative to rest |
| `LeftEyebrow`, `RightEyebrow` | 0.9 to 1.4 | eyebrow height relative to rest (1 at rest, up is above 1) |
| `LeftEyelid`, `RightEyelid` | 0.25 to 3 | eye opening relative to rest; the Face behavior shows the blink layers at 0.45 and below |
| `MouthSX`, `MouthSY` | 0.4 to 1.6 | mouth width and opening relative to rest |
| `MouthDX`, `MouthDY` | -0.17 to 0.11 | mouth position |
| `MouthShape` | 0, 1, 2 | the mouth the face makes: 0 neutral, 1 surprised, 2 smile (these are the indexes in the Face behavior's own script) |
| `*EyeGazeX/Y` | -1 to 1, with outliers | pupil position; the two eyes record the same value in every sample |

### Handle values

Per draggable handle: `Active` (1 while grabbed), `Angle/IsSet` (always 0 in the samples) and
`Translation/<space>/<axis>` for the spaces `Layer`, `mesh`, `scene` and `behavior`, axes `X` and `Y`. `Layer` and
`mesh` hold the same numbers in every sample; `scene` is in scene pixels with the origin at the centre of the frame.

## Reading and writing

`charcraft_arf::EventGraph::from_bytes` / `to_bytes`. `to_bytes(from_bytes(x)) == x` for every recording of the
sample project; a file that spells a time differently (`0.1`) reads fine and is written back with the canonical
spelling. `EventGraph::from_curves` builds a graph from channels and curves, cutting frames the way the recorder
does, which is how CharCraft exports its own takes.
