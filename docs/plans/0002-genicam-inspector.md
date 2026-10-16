# GenICam inspector and runtime controls

Status: Done (approved and implemented 2026-10-16)

## Objective

Extend the Rust/Slint Basler test app with a readable GenICam tree, measured frame rate, and safe camera controls while retaining the existing live view, PNG capture, and logs.

## Approved requirements

- Show all device information properties and discoverable GenICam nodes, including unavailable/read-only nodes, organized by category with search and node details. Include device, transport, and available stream maps.
- Display actual incoming FPS and the camera's configured FPS when the feature is readable.
- Allow editing supported operational nodes: Exposure/Auto Exposure, Gain/Auto Gain, acquisition frame rate, ROI dimensions/offsets, and PixelFormat. Only edit nodes that the SDK reports writable at the time of saving.
- Keep command nodes, trigger/acquisition controls owned by the app, and unknown or potentially persistent nodes read-only.
- Confirm every manual save. Validate input, read back the applied value, log before/after, and restart live acquisition when a change requires it.
- Do not invoke UserSetSave or persist settings. Restore edited values on normal disconnect where possible; clearly log and report restoration failures. A crash, power loss, or lost device may prevent restoration.
- Retain Windows 10/11 x64, Rust, Slint, pylon C v10, one 2D camera at a time.

## Architecture and implementation

| Item | Status |
| --- | --- |
| Fix standard widget contrast by matching Slint palette to light UI | Done; visual check pending |
| Add narrow GenApi FFI bindings and typed camera feature inspection/editing | Done |
| Extend worker commands for tree search/selection, details, FPS, apply/revert | Done |
| Add inspector screen, device metadata, tree, editor and confirmation UI | Done; visual interaction check pending |
| Update README, profile and verification record | Done |
| Build and verify with emulator and available physical camera | Done; see verification limits |

## Safety and compatibility

- Query access/availability again before writing; node availability and limits may change with selector state.
- Do not read raw port/register data or execute command nodes. Show inaccessible/unavailable nodes with their status.
- Stop the stream before applying a parameter and recreate the buffer pool when resuming, since ROI/PixelFormat can change payload size.
- Apply one node per confirmed save. Keep the original value on first change and attempt to restore in reverse order at normal disconnect. Report failures rather than claiming the camera was restored.
- No new dependencies, database, installer, or updater.

## Definition of done

The full discoverable tree and metadata are navigable; supported runtime controls can be saved with confirmation and readback; FPS updates during acquisition; existing live/capture behavior remains available; errors appear in UI/file logs; documentation records actual checks and limitations.

## Approved UI extension — 2026-10-16

The user approved a quick settings table beside live view, with the last capture below it and the full GenICam tree retained on the inspector page. The user also approved camera-side ReverseX, ReverseY, and 180-degree rotation through both nodes. The UI exposes only currently readable controls and enables editing only when pylon reports them writable. Rotation is a confirmed two-node operation with rollback on failure and normal-disconnect restoration. Hardware writes, UI interaction, and build verification for this extension remain untested.
