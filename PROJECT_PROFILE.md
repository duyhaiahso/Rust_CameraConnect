# Test-Rust — Project Profile

## Purpose and current state

This is a new Windows 10/11 x64 desktop test project for connecting to pylon-compatible Basler 2D cameras, saving and displaying still images as PNG, displaying a live view, and showing camera logs in the UI and a file. The stack is Rust and Slint. The repository uses Git.

## Confirmed local environment (2026-10-16)

- The current workstation is Windows.
- Basler pylon 26.05.0.18278 is installed, including the pylon C development headers, x64 import library/runtime, and pylon Viewer.
- Rust 1.99.1 was installed through rustup during this task. Existing PowerShell sessions may need the explicit `%USERPROFILE%\.cargo\bin\cargo.exe` path or a new shell to see it on `PATH`.

## Architecture

- Rust binary with a Slint desktop UI. The camera worker owns the pylon C v10 runtime, device, stream grabber, image converter, and command channel; Slint only receives display buffers and status/log events on its UI thread.
- Device selection uses pylon discovery, not a hard-coded camera model or transport. Only one 2D image-stream camera is opened at a time.
- Discovery combines ordinary pylon enumeration with GigE all-subnets enumeration. GigE devices omitted by ordinary enumeration appear as requiring IP/network setup and remain disconnected. While no camera is open, a confirmed dialog can apply a temporary Force IP or broadcast a persistent static IP configuration by MAC. After a successful persistent write, the worker restarts the camera IP configuration cycle, then rescans and logs the observed IP. This is an explicit exception to the session-only GenICam setting policy: persistent IP is stored on the camera only when the user chooses that separate action.
- Live view uses continuous free-run acquisition with a small buffer pool. It drains older completed buffers and sends a reduced-size image to Slint to keep the display current; PNG capture retains sensor resolution. The application records and restores trigger/acquisition values it changes during normal disconnect.
- An application-side orientation control advances clockwise 0°/90°/180°/270° with one repeatable 90° button, alongside horizontal/vertical mirroring after camera-side features. The worker applies each queued rotation or flip action in order, so rapid presses are not lost. Preview transformation works on reduced-size RGB output; PNG transformation uses the full-resolution frame and swaps dimensions for quarter-turns. The app transform resets on disconnect and does not write to the camera.
- PNG files are saved through pylon image persistence under the user's Pictures folder. Application event/error logs are daily JSONL files under LocalAppData, with about six months of retention.
- The GenICam inspector reads device information and all discoverable device/transport/available stream nodes through the pylon C/GenApi API. It presents a searchable category tree. A narrow allowlist of operational nodes is editable only when pylon reports read/write access; confirmed edits are read back and logged. Monitor places live view in its own row with an actual-FPS overlay; a camera parameter table and an application orientation panel sit below it. Capture preview and logs are on a separate tab. ReverseX and ReverseY can be edited individually or together for 180-degree camera rotation. Normal disconnect attempts to restore their initial values.
- Monitor has semantic action-button colors and a camera ROI ratio selector. The selector offers Free plus common fixed ratios, computes the largest exact Width/Height pair from current camera limits and increments, and centers the crop on apply. A locked ratio couples Width and Height edits from both quick settings and the GenICam inspector. ROI dimensions and offsets are written as one recoverable operation after stopping live acquisition. The ratio lock resets on disconnect; ROI values are restored on normal disconnect where possible.
- The Slint window uses preferred 1120×760 and minimum 800×600 logical sizes. Monitor switches its control panels from a row to a column below 920 px width and scrolls when vertical space is limited. The GenICam detail pane scrolls independently at short heights.
- Actual FPS is counted from successfully received frames; camera-reported FPS comes from a readable GenICam frame-rate feature when available.
- There is no database, authentication, installer, updater, or background service in this test project.

## Verification boundary

See [VERIFICATION.md](VERIFICATION.md) for the checks performed on this workstation. No compatibility claim for other physical cameras or transports follows from those checks.

The approved implementation scope is recorded in [docs/plans/0001-basler-camera-test.md](docs/plans/0001-basler-camera-test.md).
The inspector extension is recorded in [docs/plans/0002-genicam-inspector.md](docs/plans/0002-genicam-inspector.md).
The application image orientation extension is recorded in [docs/plans/0003-image-orientation.md](docs/plans/0003-image-orientation.md).

## Implementation constraints

- Keep camera acquisition off the Slint UI thread so the interface stays responsive.
- Treat camera buffers and SDK resources as owned objects with explicit cleanup on stop, disconnect, and application exit.
- Do not infer physical-camera compatibility from the installed SDK alone; confirm against the actual camera and driver.

## References

- [Basler pylon C programming guide](https://docs.baslerweb.com/pylonapi/c/programmingguide)
- [Slint Rust documentation](https://docs.slint.dev/latest/docs/rust/slint/)

This profile records the approved test scope and the implementation in this repository.
