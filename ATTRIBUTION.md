# Attribution

Third-party code copied into this repository and bundled assets. Dependencies used through Cargo keep their own
licenses and are not listed here.

| Path | What | Source | License |
|---|---|---|---|
| `crates/tracking/src/pipeline.rs` | Face detector anchors, weighted NMS and the rotated-crop sampling, ported to our lints | EffectCraft `crates/segment/src/{mediapipe,face}.rs` (storytold/effectcraft), itself following MediaPipe's published face landmarker graph | MIT OR Apache-2.0 |

## Models (downloaded on first use, never committed)

`crates/tracking/src/model.rs` lists each model with its source, licence and SHA-256; the store writes a `NOTICE.txt`
beside every installed file.

| Model | Authors | Source | License |
|---|---|---|---|
| MediaPipe BlazeFace short-range detector (ONNX) | Google LLC (MediaPipe); conversion by Fernando Tonon | huggingface.co/fernandotonon/QtMeshEditor-blazeface-onnx | Apache-2.0 |
| MediaPipe Face Mesh V2, 478 landmarks (ONNX) | Google LLC (MediaPipe); conversion by naklitechie | huggingface.co/naklitechie/face-landmarks-onnx | Apache-2.0 |
