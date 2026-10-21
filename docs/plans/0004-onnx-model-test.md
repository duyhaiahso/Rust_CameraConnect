# ONNX model test

Status: Done (approved and implemented 2026-10-20)

## Objective

Run the supplied one-class YOLO11 segmentation checkpoint against the Basler live stream, show the number of `Object` instances in the latest processed frame, and measure processing time and processed FPS. This is a test tool; it does not apply product targets or Pass/Fail rules.

## Current state

- Rust/Slint receives RGB camera frames in a dedicated pylon worker. Preview conversion is capped near 15 FPS and acquisition drops old frames.
- The checkpoint is outside the public repository. Its archive metadata identifies Ultralytics 8.3.176, YOLO11 nano segmentation, one `Object` class, and training image size 1600.
- This workstation has an Intel integrated GPU and no Python AI packages. Sample BMP files near the checkpoint have a 4500 x 800 format.

## Approved design

1. Add an in-app `.pt` to `.onnx` export action. The original checkpoint and exported model remain outside the repository. An isolated local Python environment supplies the conversion dependencies. There is no ONNX to PT conversion.
2. Add ONNX Runtime CPU inference in Rust. Load a selected ONNX model, validate its input/output format, letterbox RGB frames, decode the YOLO11 segmentation prediction head, apply confidence filtering and NMS, and count instances of `Object`.
3. Run inference independently of pylon acquisition and Slint. Keep at most the newest pending frame so slow inference cannot accumulate camera latency. Pair each result and annotated preview with its source frame and show processing milliseconds plus measured inference FPS separately from camera FPS.
4. Add compact model controls and results near the live view, with English/Vietnamese labels and clear loading/error/empty states. Stop/reload/unload inference cleanly.
5. Compare ONNX and original checkpoint on supplied BMP samples, run the requested build, and document setup and measured limitations.

## Dependencies and risks

- Rust: ONNX Runtime binding and a Windows native file chooser. These are new project dependencies because Slint and pylon do not run ONNX or open native model files.
- Conversion only: isolated Python with PyTorch, Ultralytics, ONNX tooling. The application does not need Python to run an already exported ONNX model.
- CPU inference at 1600 pixels may be slower than camera FPS. The UI must report this accurately and keep the camera view current.
- The model archive contains AGPL-3.0 metadata. Model artifacts and sample images remain outside the public Git repository; distribution/licensing of a later production model is a separate decision.
- ONNX output semantics and model parity must be checked against the actual checkpoint and sample images. If the export shape differs from the expected YOLO11 segmentation head, adapt the decoder without changing the counting rule.

## Work items

- Done: isolated export environment, conversion action, and ONNX artifact.
- Done: Rust inference pipeline and result annotation.
- Done: Slint controls and metrics.
- Done: sample parity, build, and documentation. Live UI interaction and physical-camera inference remain unverified.

## Definition of Done

- In-app export preserves the original `.pt` and yields a usable ONNX file outside Git.
- Live camera remains responsive while inference operates; count refers to one processed frame and old frames are dropped.
- Count, processing time, inference FPS, and visible detections are shown with useful error states.
- Original checkpoint and ONNX agree on sample-image counts under the same thresholds, or the discrepancy is identified and reported.
- The requested Windows build completes; any physical-camera validation limits are reported.
