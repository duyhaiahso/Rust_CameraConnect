use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

pub fn converter_python() -> Result<PathBuf, String> {
    let local = std::env::var_os("LOCALAPPDATA")
        .ok_or_else(|| "LOCALAPPDATA is not available".to_owned())?;
    let python = PathBuf::from(local)
        .join("BaslerCameraTest")
        .join("model-converter")
        .join("Scripts")
        .join("python.exe");
    if python.is_file() {
        Ok(python)
    } else {
        Err(
            "Model converter is not installed. Run tools/setup_model_converter.ps1 first."
                .to_owned(),
        )
    }
}

fn destination(source: &Path) -> Result<PathBuf, String> {
    let parent = source
        .parent()
        .ok_or_else(|| "Model file has no parent directory".to_owned())?;
    let stem = source
        .file_stem()
        .and_then(|name| name.to_str())
        .ok_or_else(|| "Model filename is not valid Unicode".to_owned())?;
    let first = parent.join(format!("{stem}.onnx"));
    if !first.exists() {
        return Ok(first);
    }
    for index in 2..=999 {
        let path = parent.join(format!("{stem}-{index}.onnx"));
        if !path.exists() {
            return Ok(path);
        }
    }
    Err("Too many exported ONNX files in the model folder".to_owned())
}

pub fn export(source: &Path) -> Result<PathBuf, String> {
    if !source.is_file() || source.extension().and_then(|value| value.to_str()) != Some("pt") {
        return Err("Select an existing .pt model file".to_owned());
    }
    let python = converter_python()?;
    let output_path = destination(source)?;
    let mut command = Command::new(python);
    command
        .arg("-c")
        .arg(include_str!("../tools/export_onnx.py"))
        .arg(source)
        .arg(&output_path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let result = command
        .output()
        .map_err(|error| format!("Could not start ONNX exporter: {error}"))?;
    if !result.status.success() {
        let stderr = String::from_utf8_lossy(&result.stderr);
        let detail = stderr.lines().rev().take(8).collect::<Vec<_>>();
        return Err(format!(
            "ONNX export failed: {}",
            detail.into_iter().rev().collect::<Vec<_>>().join(" | ")
        ));
    }
    let size = std::fs::metadata(&output_path)
        .map_err(|error| format!("ONNX output is missing: {error}"))?
        .len();
    if size == 0 {
        return Err("ONNX exporter produced an empty file".to_owned());
    }
    Ok(output_path)
}
