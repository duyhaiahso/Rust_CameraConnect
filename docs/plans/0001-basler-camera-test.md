# Basler camera test — approved scope

Status: Done (2026-10-16)

## Objective

Create a Windows 10/11 x64 desktop test application in Rust and Slint for one pylon-compatible Basler 2D camera at a time.

## Requirements and decisions

- Discover cameras through pylon so models and connection interfaces are not hard-coded. The installed pylon transport drivers and physical hardware determine actual availability.
- Connect/disconnect, show live view, capture a still image, display it, and save it as PNG.
- Temporarily use continuous free-run acquisition. Restore changed camera settings on normal disconnect.
- Show camera events/errors in the UI and save application logs to a file.
- Use the local pylon C SDK. Keep camera operations on a worker thread and the Slint UI on its UI thread.

## Scope boundary

No database, authentication, installer, updater, 3D-specific workflow, or multi-camera simultaneous acquisition. Other physical camera models/transports require testing with their respective hardware and drivers.

## Work items

| Item | Status |
| --- | --- |
| Inspect empty workspace, local toolchain, and pylon installation | Done |
| Scaffold Rust/Slint project and pylon C FFI | Done |
| Implement discovery, connection, free-run acquisition, conversion, cleanup | Done |
| Implement live view, PNG capture/display, and camera status/log UI | Done |
| Add daily file logs, retention, settings, and setup documentation | Done |
| Verify build, emulator, and available physical camera | Done |

## Risk and recovery

- Camera trigger/acquisition values are restored on normal disconnect; abnormal termination can leave changed runtime settings. Reopen the camera in pylon Viewer and restore settings if needed.
- Unsupported image formats and payloads above the test-app limit return a visible error.
- The test uses the pylon C v10 SDK present on the workstation. Rebuild with matching SDK/runtime if the installation changes.

See [VERIFICATION.md](../../VERIFICATION.md) for actual checks and limitations.
