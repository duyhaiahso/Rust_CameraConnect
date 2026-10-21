mod camera;
mod clock;
mod files;
mod genicam;
mod inference;
mod model_export;
mod network;
mod pylon_ffi;
mod roi;
mod transform;
mod worker;

use slint::ComponentHandle;
use std::sync::mpsc;
use std::time::{Duration, Instant};

slint::include_modules!();

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|arg| arg == "--list-cameras") {
        let runtime = camera::Runtime::new()?;
        let (cameras, warning) = runtime.enumerate_with_gige()?;
        if let Some(warning) = warning {
            eprintln!("{warning}");
        }
        println!("Detected {} pylon camera(s)", cameras.len());
        for camera in cameras {
            println!("{}", camera.label);
        }
        return Ok(());
    }
    if let Some(position) = args.iter().position(|arg| arg == "--infer-image") {
        let model = args
            .get(position + 1)
            .ok_or("--infer-image requires a model path")?;
        let image = args
            .get(position + 2)
            .ok_or("--infer-image requires an image path")?;
        let output = args.get(position + 3).map(std::path::Path::new);
        return inference::infer_image(model.as_ref(), image.as_ref(), output).map_err(Into::into);
    }
    if let Some(position) = args.iter().position(|arg| arg == "--benchmark-image") {
        let model = args
            .get(position + 1)
            .ok_or("--benchmark-image requires a model path")?;
        let image = args
            .get(position + 2)
            .ok_or("--benchmark-image requires an image path")?;
        let runs = args
            .get(position + 3)
            .ok_or("--benchmark-image requires a run count")?
            .parse::<usize>()?;
        return inference::benchmark_image(model.as_ref(), image.as_ref(), runs)
            .map_err(Into::into);
    }
    if let Some(position) = args.iter().position(|arg| arg == "--export-onnx") {
        let model = args
            .get(position + 1)
            .ok_or("--export-onnx requires a .pt model path")?;
        let output = model_export::export(std::path::Path::new(model))?;
        println!("ONNX exported: {}", output.display());
        return Ok(());
    }
    if args.iter().any(|arg| arg == "--smoke-emulator") {
        return smoke_camera(None);
    }
    if let Some(position) = args.iter().position(|arg| arg == "--smoke-serial") {
        let serial = args
            .get(position + 1)
            .ok_or("--smoke-serial requires a serial number")?;
        return smoke_camera(Some(serial));
    }
    if args.iter().any(|arg| arg == "--inspect-emulator") {
        return inspect_camera(None);
    }
    if args.iter().any(|arg| arg == "--roundtrip-emulator") {
        return roundtrip_emulator();
    }
    if args.iter().any(|arg| arg == "--roundtrip-roi-emulator") {
        return roundtrip_roi_emulator();
    }
    if let Some(position) = args.iter().position(|arg| arg == "--inspect-serial") {
        let serial = args
            .get(position + 1)
            .ok_or("--inspect-serial requires a serial number")?;
        return inspect_camera(Some(serial));
    }

    let ui = AppWindow::new()?;
    ui.set_language_index(if files::read_vietnamese_setting() {
        1
    } else {
        0
    });
    let close_window = ui.as_weak();
    ui.window().on_close_requested(move || {
        let Some(ui) = close_window.upgrade() else {
            return slint::CloseRequestResponse::HideWindow;
        };
        if (ui.get_feature_editable() && ui.get_editor_value() != ui.get_feature_value())
            || ui.get_ip_editor_dirty()
        {
            ui.set_confirm_exit(true);
            slint::CloseRequestResponse::KeepWindowShown
        } else {
            slint::CloseRequestResponse::HideWindow
        }
    });

    let (sender, receiver) = mpsc::channel();
    let camera_window = ui.as_weak();
    let worker_sender = sender.clone();
    let camera_thread = std::thread::Builder::new()
        .name("pylon-camera".to_owned())
        .spawn(move || worker::run(receiver, worker_sender, camera_window))?;

    let send = sender.clone();
    ui.on_refresh(move || {
        let _ = send.send(worker::Command::Refresh);
    });
    let send = sender.clone();
    ui.on_select_camera(move |index| {
        if let Ok(index) = usize::try_from(index) {
            let _ = send.send(worker::Command::SelectCamera(index));
        }
    });
    let send = sender.clone();
    ui.on_configure_ip(move |index, ip, mask, gateway, persistent| {
        if let Ok(index) = usize::try_from(index) {
            let _ = send.send(worker::Command::ConfigureIp {
                index,
                ip: ip.to_string(),
                mask: mask.to_string(),
                gateway: gateway.to_string(),
                persistent,
            });
        }
    });
    let send = sender.clone();
    ui.on_connect(move |index| {
        if let Ok(index) = usize::try_from(index) {
            let _ = send.send(worker::Command::Connect(index));
        }
    });
    let send = sender.clone();
    ui.on_disconnect(move || {
        let _ = send.send(worker::Command::Disconnect);
    });
    let send = sender.clone();
    ui.on_start_live(move || {
        let _ = send.send(worker::Command::StartLive);
    });
    let send = sender.clone();
    ui.on_stop_live(move || {
        let _ = send.send(worker::Command::StopLive);
    });
    let send = sender.clone();
    ui.on_capture(move || {
        let _ = send.send(worker::Command::Capture);
    });
    let send = sender.clone();
    ui.on_refresh_features(move || {
        let _ = send.send(worker::Command::RefreshFeatures);
    });
    let send = sender.clone();
    ui.on_search_features(move |query| {
        let _ = send.send(worker::Command::SearchFeatures(query.to_string()));
    });
    let send = sender.clone();
    ui.on_select_feature(move |id| {
        let _ = send.send(worker::Command::SelectFeature(id.to_string()));
    });
    let send = sender.clone();
    ui.on_toggle_category(move |id| {
        let _ = send.send(worker::Command::ToggleCategory(id.to_string()));
    });
    let send = sender.clone();
    ui.on_apply_feature(move |value| {
        let _ = send.send(worker::Command::ApplyFeature(value.to_string()));
    });
    let send = sender.clone();
    ui.on_set_roi_ratio(move |index| {
        let _ = send.send(worker::Command::SetRoiRatio(index));
    });
    let send = sender.clone();
    ui.on_apply_max_roi(move || {
        let _ = send.send(worker::Command::ApplyMaxRoi);
    });
    let send = sender.clone();
    ui.on_preview_roi_value(move |value| {
        let _ = send.send(worker::Command::PreviewRoiValue(value.to_string()));
    });
    let send = sender.clone();
    ui.on_rotate_180(move |enabled| {
        let _ = send.send(worker::Command::Rotate180(enabled));
    });
    let send = sender.clone();
    ui.on_rotate_image_90(move || {
        let _ = send.send(worker::Command::RotateImage90);
    });
    let send = sender.clone();
    ui.on_toggle_image_horizontal(move || {
        let _ = send.send(worker::Command::ToggleImageHorizontal);
    });
    let send = sender.clone();
    ui.on_toggle_image_vertical(move || {
        let _ = send.send(worker::Command::ToggleImageVertical);
    });
    let send = sender.clone();
    ui.on_reset_image_transform(move || {
        let _ = send.send(worker::Command::ResetImageTransform);
    });
    let weak = ui.as_weak();
    ui.on_pick_pt_model(move || {
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("PyTorch model", &["pt"])
            .pick_file()
            && let Some(ui) = weak.upgrade()
        {
            ui.set_model_pt_path(path.to_string_lossy().into_owned().into());
        }
    });
    let send = sender.clone();
    ui.on_export_model(move |path| {
        let _ = send.send(worker::Command::ExportModel(path.to_string().into()));
    });
    let send = sender.clone();
    ui.on_pick_onnx_model(move || {
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("ONNX model", &["onnx"])
            .pick_file()
        {
            let _ = send.send(worker::Command::LoadModel(path));
        }
    });
    let send = sender.clone();
    ui.on_unload_model(move || {
        let _ = send.send(worker::Command::UnloadModel);
    });
    let send = sender.clone();
    ui.on_select_model_source(move |index| {
        let _ = send.send(worker::Command::SetModelSource(index));
    });
    let send = sender.clone();
    ui.on_pick_model_image(move || {
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("Image", &["bmp", "png", "jpg", "jpeg"])
            .pick_file()
        {
            let _ = send.send(worker::Command::TestModelImage(path));
        }
    });
    let weak = ui.as_weak();
    ui.on_change_language(move |index| {
        if let Err(error) = files::save_vietnamese_setting(index == 1)
            && let Some(ui) = weak.upgrade()
        {
            ui.set_status_text(format!("Could not save language setting: {error}").into());
        }
    });

    let result = ui.run();
    let _ = sender.send(worker::Command::Shutdown);
    camera_thread
        .join()
        .map_err(|_| "Camera worker stopped unexpectedly")?;
    result?;
    Ok(())
}

fn smoke_camera(serial: Option<&str>) -> Result<(), Box<dyn std::error::Error>> {
    let runtime = camera::Runtime::new()?;
    let selected = runtime
        .enumerate()?
        .into_iter()
        .find(|camera| match serial {
            Some(serial) => camera.serial == serial,
            None => camera.device_class == "BaslerCamEmu",
        })
        .ok_or("Selected camera not found; use --list-cameras first")?;
    let mut device = runtime.connect(&selected.key)?;
    let converter = camera::Converter::new()?;
    let mut stream = camera::Stream::start(&device)?;
    let deadline = Instant::now() + Duration::from_secs(5);
    let frame = loop {
        if let Some(frame) = stream.next_frame(&converter, true)? {
            break frame;
        }
        if Instant::now() >= deadline {
            return Err("Timed out waiting for a camera frame".into());
        }
    };
    let tree = genicam::inspect(&device, Some(&stream));
    let started = Instant::now();
    let initial_count = stream.received_frames();
    while started.elapsed() < Duration::from_secs(1) {
        let _ = stream.next_frame(&converter, false)?;
    }
    let measured_fps =
        (stream.received_frames() - initial_count) as f64 / started.elapsed().as_secs_f64();
    let stop_errors = stream.stop();
    if !stop_errors.is_empty() {
        return Err(stop_errors.join("; ").into());
    }
    let safe_serial: String = selected
        .serial
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '_' })
        .collect();
    let output = std::env::current_dir()?
        .join("target")
        .join(format!("camera-smoke-{safe_serial}.png"));
    frame.save_png(&output)?;
    let close_errors = device.close();
    if !close_errors.is_empty() {
        return Err(close_errors.join("; ").into());
    }
    println!(
        "Camera capture: {}x{} -> {} (FPS {:.1}, GenICam nodes {})",
        frame.width,
        frame.height,
        output.display(),
        measured_fps,
        tree.nodes.len()
    );
    Ok(())
}

fn inspect_camera(serial: Option<&str>) -> Result<(), Box<dyn std::error::Error>> {
    let runtime = camera::Runtime::new()?;
    let selected = runtime
        .enumerate()?
        .into_iter()
        .find(|camera| match serial {
            Some(serial) => camera.serial == serial,
            None => camera.device_class == "BaslerCamEmu",
        })
        .ok_or("Selected camera not found; use --list-cameras first")?;
    let mut device = runtime.connect(&selected.key)?;
    let properties = genicam::device_properties(&device)?;
    let tree = genicam::inspect(&device, None);
    println!("{}", selected.label);
    println!("Device properties: {}", properties.len());
    println!("GenICam nodes: {}", tree.nodes.len());
    println!(
        "Camera FPS feature: {}",
        genicam::configured_fps(&device).unwrap_or_else(|| "unavailable".to_owned())
    );
    for warning in &tree.warnings {
        println!("Warning: {warning}");
    }
    for node in tree
        .nodes
        .iter()
        .filter(|node| {
            node.key.map == genicam::MapKind::Device
                && [
                    "Exposure",
                    "Gain",
                    "FrameRate",
                    "Width",
                    "Height",
                    "Offset",
                    "PixelFormat",
                ]
                .iter()
                .any(|part| node.key.name.contains(part))
        })
        .take(50)
    {
        let detail = genicam::detail(&device, None, &node.key)?;
        println!(
            "{} = {} ({}, editable: {})",
            node.key.name, detail.value, detail.access, detail.editable
        );
    }
    let close_errors = device.close();
    if !close_errors.is_empty() {
        return Err(close_errors.join("; ").into());
    }
    Ok(())
}

fn roundtrip_emulator() -> Result<(), Box<dyn std::error::Error>> {
    let runtime = camera::Runtime::new()?;
    let selected = runtime
        .enumerate()?
        .into_iter()
        .find(|camera| camera.device_class == "BaslerCamEmu")
        .ok_or("No pylon camera emulator found; set PYLON_CAMEMU=1")?;
    let mut device = runtime.connect(&selected.key)?;
    let key = genicam::FeatureKey {
        map: genicam::MapKind::Device,
        name: "ExposureTime".to_owned(),
    };
    let detail = genicam::detail(&device, None, &key)?;
    let original: f64 = detail.value.parse()?;
    let proposal = format!("{}", original + 100.0);
    roundtrip_feature(&device, &key, &proposal)?;
    let boolean = genicam::FeatureKey {
        map: genicam::MapKind::Device,
        name: "AcquisitionFrameRateEnable".to_owned(),
    };
    let boolean_detail = genicam::detail(&device, None, &boolean)?;
    if boolean_detail.editable {
        let proposal = if boolean_detail.value == "True" {
            "False"
        } else {
            "True"
        };
        roundtrip_feature(&device, &boolean, proposal)?;
    }
    let pixel_format = genicam::FeatureKey {
        map: genicam::MapKind::Device,
        name: "PixelFormat".to_owned(),
    };
    let pixel_detail = genicam::detail(&device, None, &pixel_format)?;
    if let Some(alternative) = pixel_detail
        .options
        .iter()
        .find(|option| *option != &pixel_detail.value)
    {
        roundtrip_feature(&device, &pixel_format, alternative)?;
    }
    let close_errors = device.close();
    if !close_errors.is_empty() {
        return Err(close_errors.join("; ").into());
    }
    Ok(())
}

fn roundtrip_feature(
    device: &camera::Device,
    key: &genicam::FeatureKey,
    proposal: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let initial = genicam::detail(device, None, key)?.value;
    let applied = genicam::apply(device, key, proposal);
    let restore_result = genicam::restore(device, key, &initial);
    let (before, after) = applied?;
    restore_result?;
    println!("{}: {before} -> {after} -> restored {before}", key.name);
    Ok(())
}

fn roundtrip_roi_emulator() -> Result<(), Box<dyn std::error::Error>> {
    let runtime = camera::Runtime::new()?;
    let selected = runtime
        .enumerate()?
        .into_iter()
        .find(|camera| camera.device_class == "BaslerCamEmu")
        .ok_or("No pylon camera emulator found; set PYLON_CAMEMU=1")?;
    let mut device = runtime.connect(&selected.key)?;
    let info = genicam::roi_info(&device)?;
    let original = info.state;
    let ratio = roi::PRESETS[1];
    let (width, height) =
        roi::largest(ratio, info.width, info.height).ok_or("Emulator has no exact 4:3 ROI")?;
    let target = genicam::RoiState {
        width,
        height,
        ..original
    };
    let result = genicam::apply_roi(&device, target, true);
    let restored = genicam::restore_roi(&device, original);
    let (_, applied) = result?;
    restored?;
    let readback = genicam::roi_info(&device)?.state;
    if readback != original {
        return Err(format!("ROI was not restored: {readback:?}").into());
    }
    let invalid = genicam::RoiState {
        width: info.width.max + 1,
        ..original
    };
    if genicam::apply_roi(&device, invalid, true).is_ok() {
        return Err("Emulator accepted an out-of-range ROI".into());
    }
    if genicam::roi_info(&device)?.state != original {
        return Err("Failed ROI attempt changed the emulator state".into());
    }
    let close_errors = device.close();
    if !close_errors.is_empty() {
        return Err(close_errors.join("; ").into());
    }
    println!("Emulator ROI: {original:?} -> {applied:?} -> restored");
    Ok(())
}
