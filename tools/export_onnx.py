"""Export an Ultralytics checkpoint without changing the original model."""

from __future__ import annotations

import argparse
import os
from pathlib import Path
import shutil
import sys
import tempfile


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("source", type=Path)
    parser.add_argument("destination", type=Path)
    args = parser.parse_args()

    source = args.source.resolve(strict=True)
    destination = args.destination.resolve()
    if source.suffix.lower() != ".pt" or destination.suffix.lower() != ".onnx":
        raise ValueError("Expected a .pt source and a .onnx destination")
    if destination.exists():
        raise FileExistsError(f"Output already exists: {destination}")
    if not destination.parent.is_dir():
        raise FileNotFoundError(f"Output directory does not exist: {destination.parent}")

    # Ultralytics otherwise installs missing packages on its own at export time.
    os.environ["YOLO_AUTOINSTALL"] = "False"
    from ultralytics import YOLO
    import onnx

    with tempfile.TemporaryDirectory(prefix="basler-onnx-") as temporary:
        staging_model = Path(temporary) / source.name
        shutil.copy2(source, staging_model)
        model = YOLO(str(staging_model), task="segment")
        if model.task != "segment":
            raise ValueError(f"Expected a segmentation checkpoint, got {model.task}")
        # Dynamic shapes preserve rectangular letterboxing for wide camera images.
        exported = Path(
            model.export(
                format="onnx",
                dynamic=True,
                nms=False,
                simplify=False,
                device="cpu",
                opset=17,
            )
        ).resolve(strict=True)
        onnx.checker.check_model(str(exported))
        shutil.move(str(exported), str(destination))
    print(f"ONNX_READY={destination}", flush=True)
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except Exception as exc:
        print(f"ONNX_EXPORT_ERROR={type(exc).__name__}: {exc}", file=sys.stderr, flush=True)
        sys.exit(1)
