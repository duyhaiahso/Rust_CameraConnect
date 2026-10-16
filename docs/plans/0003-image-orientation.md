# Application image orientation

Status: Implemented in source; verification pending (2026-10-16)

## Approved scope

- Add 0°, 90°, 180°, and 270° clockwise rotation and independent horizontal/vertical mirroring in the application for live view and PNG capture.
- Keep existing GenICam ReverseX, ReverseY, and camera-side 180° controls available and clearly separate from application controls.
- Apply application mirroring after rotation and after any camera-side transform. Keep the choice in the current camera connection only, resetting on disconnect.

## Implementation

- The worker owns transform state; UI clicks send commands and the worker reports the applied state.
- The live preview samples directly from the transformed output coordinates at display resolution, avoiding a second full-resolution image per live frame.
- PNG capture transforms the full-resolution RGB frame when needed; 90° and 270° swap width and height. Existing PNGs and the last-capture preview keep their orientation at capture time.
- No dependency or camera feature writes were added for this control.

## Remaining verification

- Build and run the Slint UI; inspect labels, layout, selection states, and all rotation/mirror combinations.
- Confirm PNG pixels and dimensions match the live preview for a known asymmetric frame, and measure live latency on the physical camera.

## Monitor layout update — 2026-10-16

The user approved a separate full-width live-view row with actual received FPS on the image. The camera parameter table and a compact icon-based application orientation panel sit below it. The last capture and camera log move to a dedicated Capture & log tab; capture, connection controls, and the full GenICam inspector remain available. UI build and interaction after this change have not been verified.
