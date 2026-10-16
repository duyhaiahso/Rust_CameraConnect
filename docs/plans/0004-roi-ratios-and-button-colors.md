# Camera ROI ratios and action colors

Status: Done (approved 2026-10-16)

## Approved scope

- Give camera actions consistent semantic colors and keep disabled actions legible.
- Add Free and common fixed aspect ratios to quick camera settings.
- Propose the largest exact resolution supported by each camera's current ROI limits and increments, then center the crop when confirmed.
- While locked, changing Width or Height computes the other dimension. Reject unsupported pairs without writing the camera. This applies from the quick editor and full GenICam inspector.
- Save Width, Height, OffsetX and OffsetY with readback and rollback; restore original ROI on normal disconnect.
- Preserve the existing 800×600 minimum window size and scrollable controls.

## Implementation

- `src/roi.rs` keeps exact integer-ratio and increment calculations separate from the pylon adapter.
- `src/genicam.rs` reads ROI bounds and applies the four-node ROI transaction.
- The worker owns the connection-only ratio lock and saved ROI snapshot. The UI asks for confirmation before applying the largest pair.
- A shared Slint action button gives connection, live, capture, save, and hazardous actions distinct colors.

## Verification

See `VERIFICATION.md`. Emulator tests do not establish behavior for every physical camera model or transport.
