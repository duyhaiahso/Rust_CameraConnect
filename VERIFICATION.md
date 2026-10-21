# Verification — 2026-10-16 (UTC+7)

Environment: Windows x64, Rust 1.99.1, Visual Studio Build Tools 2022 with Windows SDK 10.0.26100.0, Basler pylon 26.05.0.18278.

| Check | Result |
| --- | --- |
| `cargo fmt --all -- --check` | Passed. |
| `cargo check -q` | Passed after MSVC/SDK installation. |
| `cargo build -q` | Passed; `target/debug/basler-camera-test.exe` created. |
| `cargo test -q` | Passed: 1 calendar/time conversion test. |
| `--list-cameras` | Detected one physical GigE camera: acA3800-10gm, serial 22626973. |
| `PYLON_CAMEMU=1` plus `--smoke-emulator` | Captured and opened a valid 1024×1040 PNG through pylon. |
| `--smoke-serial 22626973` | Captured and opened a valid 3840×2748 PNG through the physical camera. The frame is nearly black; sampled RGB values were around 3–8, so exposure/scene brightness needs checking at the camera. |
| Hidden UI startup, 16 seconds | Process stayed running; the application log recorded discovery of one camera. The agent-started UI process was stopped after the check. |
| Rebuild after JSONL logging change | `cargo fmt --all -- --check`, `cargo test -q` (1 passed), and `cargo build -q` passed. A hidden UI process stayed running for 8 seconds, was stopped, and wrote a parseable JSONL entry with timestamp, level, source, and discovery message. |

The smoke captures are under ignored `target/`. The physical camera test exercised discovery, connection, continuous acquisition, RGB conversion, PNG save, and normal stream/device close. UI layout and button interaction were not visually inspected. Other physical camera models and transports have not been tested.

Git state at verification: `feature/basler-camera-test`, no commits and no push.

## GenICam inspector extension — 2026-10-16 (UTC+7)

| Check | Result |
| --- | --- |
| `cargo check -q`, `cargo build -q`, `cargo test -q` | Passed after the inspector changes; the one existing calendar test passed. |
| Emulator inspector | 11 device information properties and 329 GenICam nodes while idle. Its transport map returned no node map; the application reported this warning. |
| Physical acA3800-10gm inspector | 23 device information properties and 2,850 GenICam nodes while idle. ExposureTimeAbs, GainRaw, FPS, ROI, and PixelFormat nodes were readable and reported access modes. |
| Emulator parameter round trip | ExposureTime 10000.0→10100.0→10000.0; AcquisitionFrameRateEnable 1→0→1; PixelFormat Mono8→Mono10→Mono8. Each value was read back and restored. |
| Emulator stream/capture | Valid 1024×1040 PNG; 350 nodes with stream open; measured 91.7 received FPS over about one second. |
| Physical stream/capture | Valid 3840×2748 PNG; 2,947 nodes with stream open; measured 11.7 received FPS over about one second. |
| Hidden UI startup | Process remained running for ten seconds, discovered the physical camera and wrote valid JSONL logs; the agent-started process was stopped. |

The inspector screen, button contrast, category/search interaction, save confirmation, live edit/restart, and normal-disconnect restoration after an editor change have **not** been visually or interactively verified. The physical camera was only read/streamed during this extension; no physical camera feature was edited. The earlier note about the physical image being dark still applies. The CLI diagnostics validate the GenApi read path, emulator write/read/restore, stream discovery, received-frame counting, and capture path.

## Live-view latency change — 2026-10-16 (UTC+7)

The stream now drains older completed buffers before converting a frame; the worker skips conversion while a preview update is pending, and sends a display-sized image to Slint. `cargo fmt --all` completed. Build, tests, GUI interaction, physical-camera latency, and PNG capture after this change have not been checked. The earlier passing results above apply to the previous source state.

## Quick settings and camera orientation — 2026-10-16 (UTC+7)

The monitor now includes a common settings table and editor; ReverseX/ReverseY are allowed through the existing guarded GenICam write path, and a confirmed 180-degree action applies both with rollback on failure. Build, tests, UI layout/interaction, physical-camera writes, and restoration of orientation settings have not been checked after this change. Earlier passing results do not cover this source state.

## Application image orientation — 2026-10-16 (UTC+7)

The monitor now has a separate Image transform tab with 0°/90°/180°/270° clockwise rotation and horizontal/vertical mirroring for live preview and future PNG captures. This change has not been built or run; UI layout, image orientation, capture output, and physical-camera latency remain unverified. Earlier passing results do not cover this source state.

The user ran `cargo run` and reported a Slint compilation failure because `transform-rotation` conflicted with a built-in angle property. The application state property was renamed to `image-rotation-index` in both Slint components and its Rust setter. The corrected source has not yet been compiled or run.

## Monitor layout update — 2026-10-16 (UTC+7)

Live view was moved to a full-width row with an FPS overlay. Camera parameters and icon-based image orientation controls now occupy the row below; last capture and logs moved to a separate tab. This layout, the SVG icons, Slint compilation, and runtime behavior have not been verified after the change.

## Responsive window — 2026-10-16 (UTC+7)

The fixed `Window.width`/`height` values were replaced with preferred 1120×760 and minimum 800×600 sizes. Monitor now uses a flex layout to put its controls in one row or a column depending on width, with vertical scrolling when height is limited; GenICam details gained a scrollable pane. Only source inspection was performed for this change. Slint compilation, resizing at 800×600 and larger sizes, keyboard/mouse scrolling, and camera operation remain unverified.

## Image orientation tooltip compile fix — 2026-10-16 (UTC+7)

The user ran `cargo run` and reported seven Slint type errors in `ui/image_orientation.slint`: dynamic `string` values were assigned to `Tooltip.text`, which requires `styled-text`. All seven localized tooltip branches now use `@markdown` to produce `styled-text`. The source was inspected after the edit; compilation and runtime behavior have not yet been verified for this fix.

## Responsive layout binding-loop fix — 2026-10-16 (UTC+7)

The user's next `cargo run` no longer reported tooltip type errors. It reported a Slint binding loop between the Monitor `ScrollView` content width and the flex layout, plus height-loop warnings in the Window and Inspector. The Monitor scroll view now sits in a container with explicit zero layout constraints, while the Inspector uses a fixed 72px device-info height. Redundant parent-size bindings were removed from layouts that already fill their parent, including the Window layout, Inspector, and quick controls. Only source inspection was performed after this edit. Slint compilation, window resizing, and camera operation remain unverified.

## Build and runtime check after responsive layout fix — 2026-10-16 (UTC+7)

The user explicitly requested a full build check. A fresh `cargo check --locked` reproduced the remaining Slint width binding loop. Removing the Monitor `ScrollView.content-width` binding and sizing its content layout from `visible-width` resolved the loop. Fifteen Clippy warnings were fixed without changing camera workflow decisions.

| Check | Result |
| --- | --- |
| `cargo fmt --all -- --check` | Passed. |
| `cargo check --locked` | Passed after the final edits. |
| `cargo clippy --locked --all-targets -- -D warnings` | Passed with no warnings. |
| `cargo build --locked` | Passed; debug executable created. |
| `cargo test --locked` | Passed: 1 calendar/time conversion test. |
| `--list-cameras` | Detected one physical GigE camera: acA1300-60gm. The camera was not opened. |
| `PYLON_CAMEMU=1` plus `--smoke-emulator` | Captured a PNG through pylon; the saved 1024×1040 file was opened successfully. |
| Hidden UI startup | Process stayed running for five seconds and was then stopped. It was confirmed no longer running. |

The current Slint UI compiled and started without a binding-loop failure. Interactive controls, visual layout at 800×600, live view, image orientation, camera parameter writes, and the physical camera's acquisition path were not exercised in this final check.

## Single repeatable rotation control — 2026-10-16 (UTC+7)

The four rotation buttons were replaced by one 90° clockwise button with a current-angle readout. Rotation, flip, and reset now send ordered actions to the camera worker, which owns the application transform state. This prevents rapid clicks from sending repeated stale absolute angles.

| Check | Result |
| --- | --- |
| `cargo check --locked` | Passed. |
| `cargo build --locked --target-dir .\target\verification` | Passed; new executable created under `target\verification\debug\`. |
| `cargo clippy --locked --all-targets -- -D warnings` | Passed. |
| `cargo test --locked` | Passed: 1 calendar/time conversion test. |

The ordinary `cargo build --locked` could not replace `target\debug\basler-camera-test.exe` because a separate process was running that file. That process was left untouched. The new executable in `target\verification\debug\` has not been interactively tested; repeated button presses, live preview, and PNG orientation have not been checked with a running UI or physical camera after this change.

## ROI ratios and colored actions — 2026-10-16 (UTC+7)

| Check | Result |
| --- | --- |
| `cargo fmt --all -- --check` | Passed. |
| `cargo clippy --locked --all-targets --target-dir .\target\verification -- -D warnings` | Passed. |
| `cargo test --locked --target-dir .\target\verification` | Passed: 4 tests, including exact ratio, steps, and invalid pair cases. |
| `cargo build --locked --target-dir .\target\verification` | Passed; `target\verification\debug\basler-camera-test.exe` created. |
| `PYLON_CAMEMU=1` plus `--roundtrip-roi-emulator` | Applied 4:3 centered ROI 4096×3072 from 1024×1040, restored original Width/Height/OffsetX/OffsetY, then rejected an out-of-range Width without changing the camera state. |
| UI window launch | The new executable opened its own Basler Camera Test window; the agent-started process was stopped afterward. |

The Windows screenshot helper timed out twice, so button appearance, scrolling, resizing at 800×600, and complete UI interaction could not be visually verified. The physical camera was not opened or written during this change. The emulator proves this ROI transaction on its node map only; model-specific increments, offset access, and device behavior still need a physical-camera check. The original `target\debug` user process was left untouched.

## Compact action buttons — 2026-10-16 (UTC+7)

The user supplied a screenshot showing colored action buttons stretched across most of the window. `ActionButton` now has zero horizontal stretch so each button uses its text-based preferred width; the camera selector and status area continue to occupy spare space. `cargo build --locked --target-dir .\target\verification` passed. The revised layout has not been visually inspected in a running window.

## Ratio selector lock fix — 2026-10-16 (UTC+7)

The ratio selector had been disabled unless Width, Height, OffsetX, and OffsetY were all writable at the instant the parameter table refreshed. That was stricter than selecting a ratio, which changes application state only. The selector now stays enabled while a camera is connected and no edit is pending; ROI write access is still checked by pylon after stopping live acquisition for a save. The user can select a preset even if no exact size is supported; the panel explains the case and keeps Apply max disabled. Read-only inspection of a physical acA1300-60gm showed Width, Height, OffsetX, and OffsetY readable and writable while idle. `cargo build --locked --target-dir .\target\verification`, Clippy with `-D warnings`, and format check passed. GUI selection during live view and camera ROI writes were not exercised for this fix.

## GigE all-subnets discovery and IP setup — 2026-10-16 (UTC+7)

- `cargo build --locked` passed after the GigE discovery and IP dialog changes.
- `cargo clippy --all-targets -- -D warnings` passed.
- `cargo test` passed six tests, including valid-subnet and invalid-mask/IP checks.
- `cargo run -- --list-cameras` used the new combined scan and detected a physical acA1300-60gm with its network properties. This was read-only.
- No camera with a wrong subnet was present for a physical discovery check. Neither temporary nor persistent IP writes were sent to a physical camera. The dialog was compiled by Slint, but its interactive appearance and controls were not tested in a running window.
- The built UI process remained running for three seconds during a hidden startup check and was then stopped by the agent. This does not establish interactive dialog behavior.

## Persistent GigE IP activation fix — 2026-10-16 (UTC+7)

The application log showed that pylon accepted a persistent IP command for the selected camera, but the camera was not found at the target address after rescanning. A later Force IP command made it reachable. Basler's installed `Utility_IpConfig` sample restarts the camera IP configuration cycle after a successful broadcast write; this step was missing in the app and has been added for the persistent path. If restarting fails, the app reports that the persistent settings may have been stored and may require a camera power cycle.

`cargo fmt --all`, `cargo build --locked`, `cargo test --locked` (6 passed), and `cargo clippy --locked --all-targets -- -D warnings` passed. The read-only `--list-cameras` command still detected the physical acA1300-60gm. No persistent IP write or camera power-cycle test was performed after this fix, so retention across power cycles remains unverified.

## ONNX model test — 2026-10-20 (UTC+7)

- Inspected the checkpoint archive without unpickling it: Ultralytics 8.3.176, YOLO11 nano segmentation, one `Object` class, training image size 1600.
- Installed an isolated CPU PyTorch/Ultralytics/ONNX converter environment under LocalAppData. Exported the supplied checkpoint to dynamic ONNX outside Git and validated it with `onnx.checker`.
- `cargo build --target-dir target\codex-model-test` passed. A normal `cargo build` compiled the sources but could not replace `target\debug\basler-camera-test.exe` while the user's existing app instance held it open; that process was left untouched.
- PyTorch checkpoint on sample BMP (4) and (6): counts 68 and 71 at confidence 0.25, IoU 0.7, image size 1600.
- The Rust ONNX CLI on the same samples: counts 68 and 71, with 117 ms and 121 ms total processing time in the debug build. These timings include Rust preprocessing and postprocessing and are single-image observations, not a sustained FPS benchmark.
- The same Rust export path created a numbered ONNX file when the first ONNX already existed; the numbered model loaded and counted 68 on sample (4). The original checkpoint and first ONNX file were not overwritten.
- The new Slint component compiled. The live UI, file picker, marked preview, sustained inference FPS, and physical-camera interaction have **not** been visually or interactively checked in this run.
- The Rust CLI saved a marked copy of sample (4) under ignored `target\codex-model-test\`; visual inspection showed red center markers aligned with the detected row of objects. This checks the annotation helper on a still image, not the Slint image layout.
- The newly built executable opened in a hidden window, remained running for four seconds, and the agent-started process was stopped. The user's already-running application process was left untouched.
- A second numbered ONNX file (`V1_21-04-26_0.98-2.onnx`) was generated while checking that export does not overwrite the first one. Automatic cleanup of that external test artifact was blocked by the tool policy; it remains beside the original model.
- After the final source changes, `cargo fmt`, `cargo build --locked --target-dir target\codex-model-test`, `git diff --check`, and one more Rust ONNX run on sample (4) passed; the last run counted 68 objects.

## In-app single-image model input — 2026-10-20 (UTC+7)

- Added BMP, PNG, and JPEG selection in the Model test panel, using the same loaded ONNX session and inference worker as live view. Image mode blocks new live inference submissions while preserving camera acquisition.
- `cargo fmt --all` and `cargo build --locked --target-dir target\codex-model-test` passed after the change.
- The ignored integration test `selected_image_runs_without_a_camera`, run with the external ONNX and sample BMP (4), passed: a missing image returned an error without unloading the model, then the valid image counted 68 objects, produced a nonempty marked preview, and did not report a single-image FPS.
- The built app launched and exposed a Basler Camera Test window, but the Windows capture helper timed out twice. The file picker and Slint display were not visually verified. The agent-started test process is stopped after this check; the user's app process was not stopped.
- Follow-up on the reported sample-image error: `Full_L1_V1 (1).bmp` is 0 bytes, while sample (4) is 10,800,054 bytes. The app now reports a localized empty-image message in red and keeps the model loaded. `cargo fmt --all`, `cargo build --locked --target-dir target\codex-model-test`, and the ignored worker test passed; that test rejected the 0-byte BMP, then counted 68 on sample (4). The UI message color/layout still needs a visual check.

## Separate ONNX model-call time — 2026-10-20 (UTC+7)

- Added a second metric around `session.run`, separate from total frame processing time, for live frames and selected images. The panel uses two compact metric rows.
- `cargo fmt --all`, `cargo build --locked --target-dir target\codex-model-test`, and the ignored external-image worker test passed. The test checks that model time is positive and no greater than total processing time while preserving count 68.
- One debug CLI run on sample BMP (4) printed count 68, model call 44 ms, total processing 106 ms. This is a single observation, not a throughput benchmark.
- The updated two-row Slint layout compiled, but the display at 800×600 and the physical-camera live metrics were not visually checked.

## ONNX processing latency and live scheduling — 2026-10-21 (UTC+7)

- Before this change, three debug runs on the 4500×800 BMP (4) took 126–129 ms per complete frame: input preparation about 60 ms, ONNX call 48–52 ms, decoding about 1 ms, annotation about 8 ms, and preview about 8 ms. The release build took about 56–62 ms. These timings include the marked preview and exclude file loading and UI painting.
- Added a Cargo dev-profile override for this application's image loops. Afterward, three 30-frame dev benchmarks counted 68 objects consistently and measured 56.5–58.5 ms/processed frame, 41.8–43.6 ms in the ONNX call, and 17.0–17.6 FPS synthetic throughput. Two release benchmarks counted 68 and measured 55.1–55.7 ms/processed frame and 17.8–18.0 FPS. Sample BMP (6) still counted 71 in release.
- Live AI submissions are no longer tied to the roughly 15 FPS UI preview cadence. The camera worker sends an eligible frame to the existing ONNX thread before rendering its preview. A single queued frame is refreshed at most about every 33 ms; pylon wait timeout is 20 ms so AI result events can be handled sooner when the camera is idle.
- `cargo fmt --all -- --check`, `git diff --check`, the debug and release builds with `--locked --target-dir target\codex-model-test`, and the ignored no-camera image test passed. The test checked count 68, a marked preview, and the separate model/processing timing relationship.
- The 17–18 FPS figure is a repeated-image CPU benchmark, not measured live FPS. The physical camera, in-app FPS display, and end-to-end camera-to-screen delay were not tested in this change. No additional per-stage inference threads were added because the sequential model call dominates in release and splitting dependent stages would add transfer/wait overhead without measured benefit.
