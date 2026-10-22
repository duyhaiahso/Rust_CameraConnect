# Basler Camera Test

A small Windows 10/11 x64 desktop test application built with Rust, Slint, and the Basler pylon C API. It discovers pylon-compatible 2D cameras, connects to one camera, displays a live view, captures a PNG, and records application camera events and errors.

## Prerequisites

- Rust stable with the `x86_64-pc-windows-msvc` target.
- Microsoft Visual C++ Build Tools with the C++ workload and Windows SDK.
- Basler pylon with the Developer profile, x64 pylon C SDK, runtime, and the driver matching the camera interface.
- A camera that pylon can discover and that provides a 2D image stream. For a physical camera, verify connectivity in pylon Viewer first.

The project currently links to the pylon C **v10** import library. By default, `build.rs` looks for `C:\Program Files\Basler\pylon\Development\lib\x64\PylonC_v10.lib`. Set `PYLON_ROOT` to a different pylon installation root if needed. The matching pylon runtime DLL must also be available when the application starts.

## Run

```powershell
cargo run
```

To check pylon discovery without opening the UI:

```powershell
cargo run -- --list-cameras
```

To exercise discovery, connection, continuous acquisition, RGB conversion, and PNG saving with the pylon camera emulator:

```powershell
$env:PYLON_CAMEMU = "1"
cargo run -- --smoke-emulator
```

The smoke command only opens a device with class `BaslerCamEmu` and writes a PNG under `target\`.

To run the same capture path against a specific physical camera detected by `--list-cameras`, use `cargo run -- --smoke-serial <serial>`. This briefly changes its trigger/acquisition settings and restores them during normal shutdown. The output stays under `target\`.

Use **Refresh** to list cameras, select one, and choose **Connect**. **Start live** begins free-run acquisition. **Capture PNG** saves the latest recent live frame, or briefly starts acquisition if live view is stopped. **Disconnect** releases the stream and restores the trigger/acquisition settings changed by the application during normal shutdown.

The camera list also scans Basler GigE devices across local subnets. A GigE camera omitted by normal discovery is marked **WRONG SUBNET** and cannot be connected until its network address is corrected. Select it and choose **Configure IP** to see its current IP, subnet mask, gateway, MAC, and host network-interface address. Enter the new IPv4 settings and choose **Temporary IP** for this power cycle or **Save static IP** to store the address in the camera; each action asks for confirmation. Static IP disables DHCP on the camera. After a successful static write, the app restarts the camera's IP configuration cycle so the stored address becomes active without manually power cycling. The app validates the address, mask, gateway, membership with the host interface IP under the entered mask, and collisions with other discovered cameras, then rescans and reports whether the new address was found. IP changes are available only while no camera is connected. Use the network adapter's actual mask when filling the form; the app does not read or validate that mask. An IP conflict with a device outside the discovered camera list cannot be ruled out by this check. This feature does not change the Windows network adapter settings.

On **Monitor**, live view occupies its own row, with actual received FPS overlaid on the image. Its height follows the displayed image's aspect ratio up to a window-height limit; scroll down to reach the control panels. **Expand** opens the updating live preview in a larger viewer with **Fit**, **100%**, **200%**, and drag-to-pan. The viewer uses the same reduced-resolution preview as the main screen, so zoom does not restore sensor detail. Beneath it, the **Camera parameters** table shows supported Exposure, Gain, FPS, ROI, PixelFormat, Flip X, and Flip Y values. Choose **Edit** on a writable row to enter a value and confirm saving. **Rotate camera 180°** applies ReverseX and ReverseY together when both camera features are writable; the reverse action clears both. The controls use camera GenICam features, so the camera itself supplies the transformed image and PNG capture follows it. Unsupported or currently read-only features cannot be edited.

The **Ratio** selector above the parameter table starts in **Free** mode. A fixed preset (1:1, 4:3, 3:4, 16:9, 9:16, 3:2, 2:3, 5:4, 4:5, or 21:9) proposes the largest exact Width/Height pair allowed by the camera's current limits and increments. **Apply max** asks for confirmation, then changes the camera ROI and centers it. While a ratio is selected, editing Width or Height in either the quick table or GenICam inspector calculates the other dimension exactly; an unsupported pair cannot be saved. Select **Free** to edit dimensions independently. The ratio lock is only application state for the current connection. ROI writes stop live acquisition, update dimensions and offsets together, read back all four values, and attempt rollback if any step fails. Normal disconnect attempts to restore the original ROI.
You can choose a ratio while live acquisition is running; selecting it does not write the camera. **Apply max** becomes available when the current ROI limits can be read and an exact size exists. The worker stops acquisition before checking write access and applying the ROI.

The window starts at a preferred 1120×760 logical pixels and can be resized down to 800×600. The two Monitor control panels switch to a vertical arrangement below 920 px window width. The live-view height adapts to the image ratio and window height; Monitor scrolls when the live view and panels do not fit together. The GenICam details pane also scrolls so the editor remains reachable.

The **Image orientation** panel beside the parameter table has one 90° clockwise rotation button. Each press advances 0° → 90° → 180° → 270° → 0°, and the current angle appears next to the button. Separate controls mirror horizontally or vertically and reset the orientation. These controls run in the application after any camera-side ReverseX/ReverseY. They affect the live preview and the next PNG captures, including output dimensions for 90°/270°. They do not write camera settings, restart acquisition, or change previously saved PNGs. Reset clears only the application transform; disconnect also resets it.

Open **Capture & log** to see the last captured image, saved path, and camera event log. The capture action remains available from every page.

Live view discards older completed frames when the application falls behind the camera. Images sent to the window are reduced to at most 1024×768 pixels; captured PNG files retain the camera's original resolution. The displayed FPS counts received frames, which can be higher than the number of images shown by the window.

Open **Info & features** to see all device information properties and the GenICam device/transport node tree. The stream node tree appears while live acquisition is active, when pylon provides it. Search by node name, click a category to expand/collapse it, and select a node for its current value, access mode, limits, options, and description. Inaccessible nodes remain listed. The FPS display separates the rate of frames received by this application from the camera's reported frame rate when available.

The editor is intentionally limited to supported operational features: Exposure/Auto Exposure, Gain/Auto Gain, acquisition frame rate, ROI Width/Height/OffsetX/OffsetY, PixelFormat, ReverseX, and ReverseY. A node must also be writable according to pylon at the time of saving. **Save** asks for confirmation, validates the value, reads it back, and records the change in the log. During live view, acquisition is stopped and restarted so the payload buffers match any changed ROI or pixel format. The app attempts to restore the original values on normal disconnect. Commands, app-owned trigger/acquisition controls, and unknown or potentially persistent settings are view-only. No UserSet is saved.

For read-only diagnostics without opening the UI, use `cargo run -- --inspect-serial <serial>` or set `PYLON_CAMEMU=1` and use `cargo run -- --inspect-emulator`. The emulator write/read/restore check is `cargo run -- --roundtrip-emulator`.
The camera emulator ROI transaction check is `cargo run -- --roundtrip-roi-emulator`; it restores the original ROI before exit.

## Files

- PNG captures: `%USERPROFILE%\Pictures\BaslerCameraTest\`
- Daily JSONL logs: `%LOCALAPPDATA%\BaslerCameraTest\logs\` (old logs are removed after about six months)
- Language preference: `%LOCALAPPDATA%\BaslerCameraTest\language.txt`

The file log contains application camera operations and errors. It does not collect every internal pylon driver trace. Times shown in the UI, capture names, and log entries use UTC+7.

## Test a YOLO11 segmentation model

The **Model test** panel below the live view counts the `Object` instances detected in **one processed frame**. **Model (ms)** measures only the ONNX Runtime `session.run` call, from invocation until it returns the output. **Total processing (ms)** also includes input preparation, result decoding/filtering, annotation, and preview creation; neither value includes waiting for a frame or painting the UI. Choose **Red dot** or **Green box** above the result image; Green box is the default. The selected style applies to live inference, single images, folder loops, and the enlarged result. Switching styles clears the old result, reruns a selected single image, or continues on the next live/loop frame. The choice lasts for the application session. The displayed AI FPS is separate from the camera FPS shown over the live image. There is no running object total, product target, or Pass/Fail decision.

The supplied `.pt` checkpoint is exported once to ONNX. On this workstation, the converter environment has been installed at `%LOCALAPPDATA%\BaslerCameraTest\model-converter\`. On another workstation, install Python 3.11 x64 and run:

```powershell
.\tools\setup_model_converter.ps1 -PythonExe 'C:\path\to\python.exe'
```

In the app, choose a `.pt` file and press **Export ONNX**, confirm the action, then start live view. The exporter creates a new `.onnx` beside the original checkpoint and loads it automatically. If an ONNX file of that name already exists, the exporter chooses a numbered filename; it does not overwrite the original `.pt` or the existing ONNX file. **Open ONNX** loads an already exported model without Python. **Stop AI** unloads it. The app supports this checkpoint's one-class YOLO11 segmentation output; an incompatible ONNX model shows an error. Model files and sample images are outside the Git repository and are ignored by Git.

To test a saved image once, load ONNX and press **Choose image...** in the Model test panel. Select one BMP, PNG, or JPEG image. The app shows its object count, processing time, and marked preview without requiring a camera connection. The image result remains visible while camera live view continues; AI stops receiving live frames until **Live** is selected again. Switching back to **Image** reruns the last selected single image. A one-shot image does not show FPS.

For continuous testing, select an image or press **Choose folder...**, then **Start loop**. A single image is decoded once and repeatedly passed to the model. A folder is scanned at its top level for BMP, PNG and JPEG files, sorted by filename, and processed one by one; after the last file the app starts again at the first. Folder images are read on demand rather than kept in memory. Unreadable or empty images are skipped and logged once per loop session; a folder with no readable images stops with an error. The panel shows the current filename, its per-image count and bounding boxes, processing/model milliseconds, completed-image FPS, and the number of images processed since Start. The FPS of a folder loop includes file loading, whereas the processing milliseconds exclude it. **Stop loop**, **Live**, and **Stop AI** stop repetition after any running model call finishes. Results are not saved automatically.

The detection result uses a full-width row whose height follows the image aspect ratio within the Monitor scroll area. Click the result or **Zoom** to open the marked image at its original resolution. Use **Fit**, **100%**, **200%**, or **400%** and drag or scroll to inspect small marks. The detail window keeps the selected displayed frame frozen while continuous detection carries on; closing it releases the full-resolution UI image.

When an ONNX model loads, **AI input** uses its `imgsz` metadata (width × height); a fixed ONNX input uses its fixed dimensions. If neither is available, the app keeps the previous 1600×1600 fallback. For a dynamic model, enter a square size such as `704` or a width × height pair such as `960x640`, then press **Apply**. Each side must be a multiple of 32 from 64 to 2048. **Model size** restores the model-derived default. This setting lasts only for the loaded model; changing it clears the old result, reruns a selected single image, or continues an image loop with fresh results. A smaller input can change object counts, so compare against known counts before using it for production decisions.

An empty or unreadable image shows a short error in the panel and leaves the ONNX model loaded. Replace a 0-byte file with a valid image; the app cannot recover image data from an empty file.

The ONNX export uses dynamic image shapes. Inference letterboxes each RGB camera frame to the active model input size with stride-aligned padding, uses confidence 0.25 and NMS IoU 0.7, and counts up to 300 objects. The model worker keeps only the newest waiting frame, so camera acquisition is not held back by inference. Only CPU execution has been configured on this test workstation; the actual speed depends on image size and machine load. Counts on a camera with a different field of view need to be checked with representative images.

For a read-only image check without opening the UI:

```powershell
cargo run -- --infer-image 'C:\path\to\model.onnx' 'C:\path\to\sample.bmp'
```

The diagnostic `--export-onnx 'C:\path\to\model.pt'` invokes the same exporter as the UI. It needs the converter environment above. For speed measurements, prefer a release build (`cargo run --release`) after the model works in the debug build.

To measure repeated processing of one saved image, run `cargo run -- --benchmark-image '<model.onnx>' '<sample.bmp>' 30`. The command uses the model-derived input size, loads and warms the model once, then reports average ONNX call time, complete processing time, and maximum single-worker throughput. Add an optional size, for example `30 1600` or `30 960x640`, to compare sizes without changing the model. It does not include camera acquisition, pylon RGB conversion, file loading, or UI painting. Ordinary `cargo run` now optimizes this application's image-processing code even in the dev profile; use a release build for final performance measurements.

In live mode, camera acquisition and ONNX inference run on separate threads. The live preview is updated at about 15 FPS, while AI can accept newer frames independently, subject to its processing speed and the camera's actual frame rate. A single waiting AI frame is refreshed at most about every 33 ms to keep results current without accumulating a queue. The pylon wait timeout is 20 ms, after which the camera worker can handle pending AI result events. Displayed AI FPS is measured from completed results; it may be below benchmark throughput when the camera is slower or the workstation is busy.

## Scope and limitations

- One 2D image-stream camera at a time. USB3, GigE, and other pylon transports depend on the installed drivers and hardware.
- GigE all-subnets discovery is limited to networks reachable by pylon's local GigE discovery broadcasts. Devices behind routers or isolated VLANs may not appear. A persistent IP change can leave a camera unreachable if its address, mask, gateway, or host network setup is wrong; use the pylon IP Configurator to recover it if necessary.
- The test app rejects an image payload above 256 MiB to avoid allocating an excessive four-buffer pool.
- The app temporarily turns off supported acquisition/frame triggers and selects continuous acquisition. It does not save camera UserSet data. Normal disconnect restores settings it changed; a crash or power loss can prevent restoration.
- A lost device, crash, or power failure can also prevent restoration of values changed in the feature editor. Restoration failures are shown in status/logs. Camera features vary by model; a node's presence in the tree does not mean it can be edited.
- Unsupported or non-convertible pixel payloads produce an error in the camera log. Specialized 3D and frame-grabber workflows are outside this test app.
- PNG saving uses pylon's C API. Paths containing characters unsupported by the installed pylon C runtime may fail; the app reports the save error.
