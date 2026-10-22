use crate::camera::{CameraInfo, Converter, Device, Frame, Runtime, Stream};
use crate::clock::Timestamp;
use crate::files::{self, Logger};
use crate::genicam::{self, FeatureDetail, FeatureKey, FeatureTree, MapKind};
use crate::inference::{
    self, AnnotationMode, Event as InferenceEvent, InputSize, LoopInput, Runner as InferenceRunner,
    SizeSource, Source as ModelSource,
};
use crate::model_export;
use crate::network::IpSettings;
use crate::roi::{self, Ratio};
use crate::transform::ImageTransform;
use crate::{AppWindow, FeatureRow, QuickFeatureRow};
use slint::{Model, Rgb8Pixel, SharedPixelBuffer, SharedString, Weak};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub enum Command {
    Refresh,
    SelectCamera(usize),
    ConfigureIp {
        index: usize,
        ip: String,
        mask: String,
        gateway: String,
        persistent: bool,
    },
    Connect(usize),
    Disconnect,
    StartLive,
    StopLive,
    Capture,
    RefreshFeatures,
    SearchFeatures(String),
    SelectFeature(String),
    ToggleCategory(String),
    ApplyFeature(String),
    PreviewRoiValue(String),
    SetRoiRatio(i32),
    ApplyMaxRoi,
    Rotate180(bool),
    RotateImage90,
    ToggleImageHorizontal,
    ToggleImageVertical,
    ResetImageTransform,
    ExportModel(PathBuf),
    ModelExported(Result<PathBuf, String>),
    LoadModel(PathBuf),
    UnloadModel,
    SetModelSource(i32),
    TestModelImage(PathBuf),
    SelectModelFolder(PathBuf),
    StartModelLoop,
    StopModelLoop,
    SetModelInputSize(String),
    ResetModelInputSize,
    SetAnnotationMode(i32),
    OpenModelDetail,
    CloseModelDetail,
    InferenceEvent(InferenceEvent),
    Shutdown,
}

struct UiSink {
    window: Weak<AppWindow>,
    preview_queued: Arc<AtomicBool>,
    model_preview_queued: Arc<AtomicBool>,
    model_result_revision: Arc<AtomicU64>,
    displayed_model_frame: Arc<Mutex<Option<(Arc<Frame>, ImageTransform)>>>,
}

impl UiSink {
    fn new(window: Weak<AppWindow>) -> Self {
        Self {
            window,
            preview_queued: Arc::new(AtomicBool::new(false)),
            model_preview_queued: Arc::new(AtomicBool::new(false)),
            model_result_revision: Arc::new(AtomicU64::new(0)),
            displayed_model_frame: Arc::new(Mutex::new(None)),
        }
    }

    fn cameras(&self, labels: Vec<String>, selected: usize) {
        let _ = self.window.upgrade_in_event_loop(move |ui| {
            let labels: Vec<SharedString> = labels.into_iter().map(Into::into).collect();
            let model = std::rc::Rc::new(slint::VecModel::from(labels));
            let has_cameras = model.row_count() > 0;
            ui.set_cameras(model.into());
            ui.set_selected_index(if has_cameras { selected as i32 } else { -1 });
        });
    }

    fn network_camera(&self, camera: Option<&CameraInfo>) {
        let gig_e = camera.and_then(|camera| camera.gige.as_ref());
        let is_gige = gig_e.is_some();
        let reachable = camera.is_some_and(|camera| camera.reachable);
        let mac = gig_e.map(|gige| gige.mac.clone()).unwrap_or_default();
        let ip = gig_e.map(|gige| gige.ip.clone()).unwrap_or_default();
        let mask = gig_e.map(|gige| gige.mask.clone()).unwrap_or_default();
        let gateway = gig_e.map(|gige| gige.gateway.clone()).unwrap_or_default();
        let interface = gig_e
            .map(|gige| gige.interface_ip.clone())
            .unwrap_or_default();
        let _ = self.window.upgrade_in_event_loop(move |ui| {
            ui.set_selected_camera_gige(is_gige);
            ui.set_selected_camera_reachable(reachable);
            ui.set_selected_camera_mac(mac.into());
            ui.set_selected_camera_ip(ip.clone().into());
            ui.set_selected_camera_mask(mask.clone().into());
            ui.set_selected_camera_gateway(gateway.clone().into());
            ui.set_selected_camera_interface(interface.into());
            ui.set_ip_editor_address(ip.into());
            ui.set_ip_editor_mask(mask.into());
            ui.set_ip_editor_gateway(if gateway.is_empty() {
                "0.0.0.0".into()
            } else {
                gateway.into()
            });
            ui.set_ip_error("".into());
        });
    }

    fn ip_error(&self, error: String) {
        let _ = self.window.upgrade_in_event_loop(move |ui| {
            ui.set_busy(false);
            ui.set_ip_error(error.into());
        });
    }

    fn ip_error_localized(&self, english: &str, vietnamese: &str) {
        let english = english.to_owned();
        let vietnamese = vietnamese.to_owned();
        let _ = self.window.upgrade_in_event_loop(move |ui| {
            ui.set_busy(false);
            ui.set_ip_error(if ui.get_language_index() == 1 {
                vietnamese.into()
            } else {
                english.into()
            });
        });
    }

    fn ip_applied(&self) {
        let _ = self.window.upgrade_in_event_loop(move |ui| {
            ui.set_busy(false);
            ui.set_ip_editor_open(false);
        });
    }

    fn status(&self, connected: bool, live: bool, english: String, vietnamese: String) {
        let _ = self.window.upgrade_in_event_loop(move |ui| {
            ui.set_connected(connected);
            ui.set_live(live);
            ui.set_busy(false);
            ui.set_status_text(if ui.get_language_index() == 1 {
                vietnamese.into()
            } else {
                english.into()
            });
        });
    }

    fn log(&self, line: String) {
        let _ = self.window.upgrade_in_event_loop(move |ui| {
            let current = ui.get_log_text();
            let appended = if current.is_empty() {
                line
            } else {
                format!("{current}\n{line}")
            };
            let mut recent = appended.lines().rev().take(200).collect::<Vec<_>>();
            recent.reverse();
            ui.set_log_text(recent.join("\n").into());
        });
    }

    fn frame(&self, frame: &Frame, snapshot: bool, transform: ImageTransform) {
        if !snapshot && self.preview_queued.swap(true, Ordering::AcqRel) {
            return;
        }
        let pixels = display_pixels(frame, transform);
        let preview_width = pixels.width() as i32;
        let preview_height = pixels.height() as i32;
        let flag = Arc::clone(&self.preview_queued);
        if self
            .window
            .upgrade_in_event_loop(move |ui| {
                let image = slint::Image::from_rgb8(pixels);
                if snapshot {
                    ui.set_snapshot(image);
                    ui.set_has_snapshot(true);
                } else {
                    ui.set_preview(image);
                    ui.set_preview_width(preview_width);
                    ui.set_preview_height(preview_height);
                    ui.set_has_preview(true);
                    flag.store(false, Ordering::Release);
                }
            })
            .is_err()
        {
            self.preview_queued.store(false, Ordering::Release);
        }
    }

    fn preview_pending(&self) -> bool {
        self.preview_queued.load(Ordering::Acquire)
    }

    fn refresh_preview(&self, frame: &Frame, transform: ImageTransform) {
        // A queued image can still represent the previous orientation. Queue the
        // replacement after it, including when acquisition is stopped.
        self.preview_queued.store(false, Ordering::Release);
        self.frame(frame, false, transform);
    }

    fn saved(&self, path: String) {
        let _ = self.window.upgrade_in_event_loop(move |ui| {
            ui.set_saved_path(path.into());
        });
    }

    fn clear_preview(&self) {
        let _ = self.window.upgrade_in_event_loop(|ui| {
            ui.set_preview(slint::Image::default());
            ui.set_preview_width(4);
            ui.set_preview_height(3);
            ui.set_has_preview(false);
            ui.set_live_detail_open(false);
        });
    }

    fn device_info(&self, info: String) {
        let _ = self
            .window
            .upgrade_in_event_loop(move |ui| ui.set_device_info(info.into()));
    }

    fn features(&self, rows: Vec<FeatureRow>) {
        let _ = self.window.upgrade_in_event_loop(move |ui| {
            ui.set_feature_rows(std::rc::Rc::new(slint::VecModel::from(rows)).into());
        });
    }

    fn quick_features(&self, rows: Vec<QuickFeatureRow>, rotate_available: bool, rotated: bool) {
        let _ = self.window.upgrade_in_event_loop(move |ui| {
            ui.set_quick_features(std::rc::Rc::new(slint::VecModel::from(rows)).into());
            ui.set_rotate_available(rotate_available);
            ui.set_rotated_180(rotated);
        });
    }

    fn roi(&self, ratio_index: i32, available: bool, maximum: String) {
        let _ = self.window.upgrade_in_event_loop(move |ui| {
            ui.set_roi_ratio_index(ratio_index);
            ui.set_roi_available(available);
            ui.set_roi_max_text(maximum.into());
        });
    }

    fn roi_preview(&self, valid: bool, english: String, vietnamese: String) {
        let _ = self.window.upgrade_in_event_loop(move |ui| {
            ui.set_roi_input_valid(valid);
            ui.set_roi_paired_preview(if ui.get_language_index() == 1 {
                vietnamese.into()
            } else {
                english.into()
            });
        });
    }

    fn image_transform(&self, transform: ImageTransform) {
        let _ = self.window.upgrade_in_event_loop(move |ui| {
            ui.set_image_rotation_index(transform.quarter_turns as i32);
            ui.set_transform_flip_horizontal(transform.flip_horizontal);
            ui.set_transform_flip_vertical(transform.flip_vertical);
        });
    }

    fn close_quick_editor(&self) {
        let _ = self
            .window
            .upgrade_in_event_loop(|ui| ui.set_quick_editor_open(false));
    }

    fn feature_detail(&self, detail: Option<FeatureDetail>) {
        let _ = self.window.upgrade_in_event_loop(move |ui| {
            if let Some(detail) = detail {
                let meta = format!(
                    "{} / {}  ·  {}  ·  {}  ·  {}",
                    detail.key.map.label(),
                    detail.key.name,
                    detail.node_type,
                    detail.access,
                    detail.visibility
                );
                ui.set_selected_feature_key(detail.key.id().into());
                ui.set_feature_name(detail.display.into());
                ui.set_feature_meta(meta.into());
                ui.set_feature_value(detail.value.clone().into());
                ui.set_feature_description(detail.description.into());
                ui.set_feature_limits(detail.limits.into());
                let options: Vec<SharedString> =
                    detail.options.into_iter().map(Into::into).collect();
                ui.set_feature_options(std::rc::Rc::new(slint::VecModel::from(options)).into());
                ui.set_editor_value(detail.value.into());
                ui.set_feature_editable(detail.editable);
            } else {
                ui.set_selected_feature_key("".into());
                ui.set_feature_name("".into());
                ui.set_feature_meta("".into());
                ui.set_feature_value("".into());
                ui.set_editor_value("".into());
                ui.set_feature_description("".into());
                ui.set_feature_limits("".into());
                ui.set_feature_options(
                    std::rc::Rc::new(slint::VecModel::<SharedString>::default()).into(),
                );
                ui.set_feature_editable(false);
            }
            ui.set_feature_error("".into());
            ui.set_roi_input_valid(true);
            ui.set_roi_paired_preview("".into());
        });
    }

    fn feature_error(&self, error: String) {
        let _ = self.window.upgrade_in_event_loop(move |ui| {
            ui.set_feature_error(error.into());
            ui.set_busy(false);
        });
    }

    fn fps(&self, actual: String, configured: String) {
        let _ = self.window.upgrade_in_event_loop(move |ui| {
            ui.set_actual_fps(actual.into());
            ui.set_configured_fps(configured.into());
        });
    }

    fn model_status(
        &self,
        ready: bool,
        busy: bool,
        english: String,
        vietnamese: String,
        path: String,
    ) {
        let _ = self.window.upgrade_in_event_loop(move |ui| {
            ui.set_model_ready(ready);
            ui.set_model_busy(busy);
            ui.set_model_error(false);
            ui.set_model_status(if ui.get_language_index() == 1 {
                vietnamese.into()
            } else {
                english.into()
            });
            if !path.is_empty() {
                ui.set_model_path(path.into());
            }
        });
    }

    fn model_image_error(&self, english: &'static str, vietnamese: &'static str) {
        let _ = self.window.upgrade_in_event_loop(move |ui| {
            ui.set_model_busy(false);
            ui.set_model_error(true);
            ui.set_model_status(if ui.get_language_index() == 1 {
                vietnamese.into()
            } else {
                english.into()
            });
        });
    }

    fn model_size(&self, active: InputSize, default: InputSize, source: SizeSource) {
        let _ = self.window.upgrade_in_event_loop(move |ui| {
            ui.set_model_input_size(active.label().into());
            ui.set_model_default_size(default.label().into());
            ui.set_model_size_draft(active.label().into());
            ui.set_model_size_source(match source {
                SizeSource::Fallback => 0,
                SizeSource::Metadata => 1,
                SizeSource::Fixed => 2,
            });
            ui.set_model_size_error("".into());
            ui.set_model_busy(false);
        });
    }

    fn model_size_reset(&self) {
        let _ = self.window.upgrade_in_event_loop(|ui| {
            ui.set_model_input_size("".into());
            ui.set_model_default_size("".into());
            ui.set_model_size_draft("".into());
            ui.set_model_size_source(0);
            ui.set_model_size_error("".into());
        });
    }

    fn annotation_mode(&self, mode: AnnotationMode) {
        let _ = self.window.upgrade_in_event_loop(move |ui| {
            ui.set_model_annotation_mode(if mode == AnnotationMode::Box { 1 } else { 0 });
            ui.set_model_busy(false);
        });
    }

    fn model_size_error(&self, english: &'static str, vietnamese: &'static str) {
        let _ = self.window.upgrade_in_event_loop(move |ui| {
            ui.set_model_busy(false);
            ui.set_model_size_error(if ui.get_language_index() == 1 {
                vietnamese.into()
            } else {
                english.into()
            });
        });
    }

    fn model_clear_result(&self) {
        self.model_result_revision.fetch_add(1, Ordering::AcqRel);
        let displayed_frame = Arc::clone(&self.displayed_model_frame);
        let _ = self.window.upgrade_in_event_loop(move |ui| {
            if let Ok(mut frame) = displayed_frame.lock() {
                *frame = None;
            }
            ui.set_model_has_result(false);
            ui.set_model_count(0);
            ui.set_model_run_ms("—".into());
            ui.set_model_processing_ms("—".into());
            ui.set_model_processed_fps("—".into());
            ui.set_model_age_ms("—".into());
            ui.set_model_processed_images(0);
            ui.set_model_preview(slint::Image::default());
            ui.set_model_result_aspect(0.32);
            ui.set_model_detail_open(false);
            ui.set_model_detail_image(slint::Image::default());
        });
    }

    fn open_model_detail(&self) {
        let displayed = self
            .displayed_model_frame
            .lock()
            .ok()
            .and_then(|slot| slot.clone());
        let Some((frame, transform)) = displayed else {
            return;
        };
        let (width, height) = transform.dimensions(frame.width, frame.height);
        let pixels = if transform.is_identity() {
            SharedPixelBuffer::<Rgb8Pixel>::clone_from_slice(&frame.rgb, width, height)
        } else {
            let transformed = transform.apply(&frame);
            SharedPixelBuffer::<Rgb8Pixel>::clone_from_slice(&transformed.rgb, width, height)
        };
        let _ = self.window.upgrade_in_event_loop(move |ui| {
            if !ui.get_model_has_result() {
                return;
            }
            ui.set_model_detail_width(width as i32);
            ui.set_model_detail_height(height as i32);
            ui.set_model_detail_image(slint::Image::from_rgb8(pixels));
            ui.set_model_detail_open(true);
        });
    }

    fn close_model_detail(&self) {
        let _ = self.window.upgrade_in_event_loop(|ui| {
            ui.set_model_detail_open(false);
            ui.set_model_detail_image(slint::Image::default());
        });
    }

    fn model_looping(&self, looping: bool) {
        let _ = self
            .window
            .upgrade_in_event_loop(move |ui| ui.set_model_looping(looping));
    }

    fn model_source(&self, source: ModelSource, path: String) {
        let _ = self.window.upgrade_in_event_loop(move |ui| {
            ui.set_model_source_index(if source == ModelSource::Image { 1 } else { 0 });
            ui.set_model_image_path(path.into());
        });
    }

    fn model_clear_path(&self) {
        let _ = self
            .window
            .upgrade_in_event_loop(|ui| ui.set_model_path("".into()));
    }

    fn model_result(&self, result: inference::ResultFrame) {
        let is_live = result.source == ModelSource::Live;
        let coalesce = is_live || result.continuous;
        if coalesce && self.model_preview_queued.swap(true, Ordering::AcqRel) {
            return;
        }
        let revision = self.model_result_revision.load(Ordering::Acquire);
        let current_revision = Arc::clone(&self.model_result_revision);
        let source_index = if is_live { 0 } else { 1 };
        let pixels = SharedPixelBuffer::<Rgb8Pixel>::clone_from_slice(
            &result.preview.rgb,
            result.preview.width,
            result.preview.height,
        );
        let aspect = result.preview.height as f32 / result.preview.width as f32;
        let detail_frame = result.detail_frame;
        let transform = result.transform;
        let displayed_frame = Arc::clone(&self.displayed_model_frame);
        let count = result.count as i32;
        let model_ms = format!("{:.0}", result.model_ms);
        let processing = format!("{:.0}", result.processing_ms);
        let fps = result
            .processed_fps
            .map(|value| format!("{value:.1}"))
            .unwrap_or_else(|| "—".to_owned());
        let age = result
            .age_ms
            .map(|value| value.to_string())
            .unwrap_or_default();
        let image_path = result.image_path.map(|path| path.display().to_string());
        let processed_images = result.processed_images.min(i32::MAX as u64) as i32;
        let queued = Arc::clone(&self.model_preview_queued);
        if self
            .window
            .upgrade_in_event_loop(move |ui| {
                if current_revision.load(Ordering::Acquire) != revision
                    || ui.get_model_source_index() != source_index
                {
                    if coalesce {
                        queued.store(false, Ordering::Release);
                    }
                    return;
                }
                ui.set_model_count(count);
                ui.set_model_run_ms(model_ms.into());
                ui.set_model_processing_ms(processing.into());
                ui.set_model_processed_fps(fps.into());
                ui.set_model_age_ms(age.into());
                ui.set_model_processed_images(processed_images);
                if let Some(path) = image_path {
                    ui.set_model_image_path(path.into());
                }
                ui.set_model_preview(slint::Image::from_rgb8(pixels));
                ui.set_model_result_aspect(aspect);
                if let Ok(mut frame) = displayed_frame.lock() {
                    *frame = Some((detail_frame, transform));
                }
                ui.set_model_has_result(true);
                if coalesce {
                    queued.store(false, Ordering::Release);
                }
            })
            .is_err()
        {
            if coalesce {
                self.model_preview_queued.store(false, Ordering::Release);
            }
        }
    }
}

fn display_pixels(frame: &Frame, transform: ImageTransform) -> SharedPixelBuffer<Rgb8Pixel> {
    // The window never displays the camera's full sensor resolution. Keep the
    // original RGB frame for PNG capture and send a smaller image to Slint.
    let (transformed_width, transformed_height) = transform.dimensions(frame.width, frame.height);
    let factor = transformed_width
        .div_ceil(1024)
        .max(transformed_height.div_ceil(768));
    if factor <= 1 && transform.is_identity() {
        return SharedPixelBuffer::clone_from_slice(&frame.rgb, frame.width, frame.height);
    }
    let width = transformed_width.div_ceil(factor);
    let height = transformed_height.div_ceil(factor);
    let mut pixels = SharedPixelBuffer::<Rgb8Pixel>::new(width, height);
    let output_stride = width as usize * 3;
    for (y, row) in pixels
        .make_mut_bytes()
        .chunks_exact_mut(output_stride)
        .enumerate()
    {
        for (x, pixel) in row.as_chunks_mut::<3>().0.iter_mut().enumerate() {
            let (source_x, source_y) = transform.source(
                x as u32 * factor,
                y as u32 * factor,
                frame.width,
                frame.height,
            );
            let source = (source_y as usize * frame.width as usize + source_x as usize) * 3;
            pixel.copy_from_slice(&frame.rgb[source..source + 3]);
        }
    }
    pixels
}

pub fn run(receiver: Receiver<Command>, sender: Sender<Command>, window: Weak<AppWindow>) {
    let mut worker = Worker::new(UiSink::new(window), sender);
    worker.refresh();
    loop {
        let command = if worker.stream.is_some() {
            match receiver.try_recv() {
                Ok(command) => Some(command),
                Err(TryRecvError::Empty) => None,
                Err(TryRecvError::Disconnected) => Some(Command::Shutdown),
            }
        } else {
            receiver.recv().ok().or(Some(Command::Shutdown))
        };
        match command {
            Some(Command::Shutdown) => break,
            Some(command) => worker.handle(command),
            None => worker.poll_live(),
        }
    }
    let _ = worker.disconnect();
}

struct Worker {
    ui: UiSink,
    sender: Sender<Command>,
    inference: Option<InferenceRunner>,
    model_generation: u64,
    live_epoch: u64,
    model_ready: bool,
    model_input_size: InputSize,
    model_default_size: InputSize,
    model_size_source: SizeSource,
    annotation_mode: AnnotationMode,
    model_source: ModelSource,
    model_input: Option<LoopInput>,
    model_looping: bool,
    exporting_model: bool,
    logger: Option<Logger>,
    runtime: Option<Runtime>,
    cameras: Vec<CameraInfo>,
    selected_camera_index: usize,
    device: Option<Device>,
    converter: Option<Converter>,
    stream: Option<Stream>,
    last_frame: Option<Arc<Frame>>,
    last_frame_at: Option<Instant>,
    last_preview_at: Option<Instant>,
    last_received_at: Option<Instant>,
    live_started_at: Option<Instant>,
    no_frame_warned: bool,
    feature_tree: Option<FeatureTree>,
    selected_feature: Option<FeatureKey>,
    collapsed_categories: HashSet<String>,
    feature_search: String,
    original_features: Vec<(FeatureKey, String)>,
    original_roi: Option<genicam::RoiState>,
    roi_ratio_index: i32,
    fps_since: Option<Instant>,
    fps_count: u64,
    actual_fps: String,
    configured_fps: String,
    image_transform: ImageTransform,
}

impl Worker {
    fn new(ui: UiSink, sender: Sender<Command>) -> Self {
        let logger = Logger::new();
        if let Err(error) = &logger {
            ui.log(format!("File logging unavailable: {error}"));
        }
        let inference = match InferenceRunner::new(sender.clone()) {
            Ok(runner) => Some(runner),
            Err(error) => {
                ui.model_status(false, false, error.clone(), error, String::new());
                None
            }
        };
        let worker = Self {
            ui,
            sender,
            inference,
            model_generation: 0,
            live_epoch: 0,
            model_ready: false,
            model_input_size: InputSize {
                width: 1600,
                height: 1600,
            },
            model_default_size: InputSize {
                width: 1600,
                height: 1600,
            },
            model_size_source: SizeSource::Fallback,
            annotation_mode: AnnotationMode::Box,
            model_source: ModelSource::Live,
            model_input: None,
            model_looping: false,
            exporting_model: false,
            logger: logger.ok(),
            runtime: None,
            cameras: Vec::new(),
            selected_camera_index: 0,
            device: None,
            converter: None,
            stream: None,
            last_frame: None,
            last_frame_at: None,
            last_preview_at: None,
            last_received_at: None,
            live_started_at: None,
            no_frame_warned: false,
            feature_tree: None,
            selected_feature: None,
            collapsed_categories: HashSet::new(),
            feature_search: String::new(),
            original_features: Vec::new(),
            original_roi: None,
            roi_ratio_index: 0,
            fps_since: None,
            fps_count: 0,
            actual_fps: "—".to_owned(),
            configured_fps: "—".to_owned(),
            image_transform: ImageTransform::default(),
        };
        if let Some(directory) = worker
            .logger
            .as_ref()
            .map(|log| log.directory().display().to_string())
        {
            worker.log("INFO", format!("Log directory: {directory}"));
        }
        worker
    }

    fn log(&self, level: &str, message: String) {
        let timestamp = Timestamp::now();
        if let Some(logger) = &self.logger
            && let Err(error) = logger.write(&timestamp, level, &message)
        {
            self.ui.log(format!("[FILE ERROR] {error}"));
        }
        self.ui
            .log(format!("{} [{level}] {message}", timestamp.display()));
    }

    fn status(&self, english: impl Into<String>, vietnamese: impl Into<String>) {
        self.ui.status(
            self.device.is_some(),
            self.stream.is_some(),
            english.into(),
            vietnamese.into(),
        );
    }

    fn failure(&self, error: String) {
        self.log("ERROR", error.clone());
        self.status(error.clone(), error);
    }

    fn ensure_runtime(&mut self) -> Result<&Runtime, String> {
        if self.runtime.is_none() {
            self.runtime = Some(Runtime::new()?);
        }
        Ok(self.runtime.as_ref().expect("runtime was initialized"))
    }

    fn refresh_features(&mut self) {
        let Some(device) = self.device.as_ref() else {
            return;
        };
        let tree = genicam::inspect(device, self.stream.as_ref());
        let count = tree.nodes.len();
        for warning in &tree.warnings {
            self.log("WARN", format!("GenICam: {warning}"));
        }
        self.feature_tree = Some(tree);
        self.configured_fps = genicam::configured_fps(device).unwrap_or_else(|| "—".to_owned());
        self.ui
            .fps(self.actual_fps.clone(), self.configured_fps.clone());
        self.show_feature_rows();
        self.show_quick_features();
        if self.selected_feature.as_ref().is_some_and(|key| {
            self.feature_tree
                .as_ref()
                .is_some_and(|tree| tree.nodes.iter().any(|node| &node.key == key))
        }) {
            self.show_selected_detail();
        } else {
            self.selected_feature = None;
            self.ui.feature_detail(None);
        }
        self.log("INFO", format!("Loaded {count} GenICam node(s)"));
    }

    fn show_quick_features(&self) {
        const GROUPS: &[(&str, &str, &[&str])] = &[
            ("Exposure Auto", "Phơi sáng tự động", &["ExposureAuto"]),
            (
                "Exposure",
                "Phơi sáng",
                &["ExposureTime", "ExposureTimeAbs", "ExposureTimeRaw"],
            ),
            ("Gain Auto", "Gain tự động", &["GainAuto"]),
            ("Gain", "Gain", &["Gain", "GainAbs", "GainRaw"]),
            (
                "FPS Enable",
                "Bật giới hạn FPS",
                &["AcquisitionFrameRateEnable"],
            ),
            (
                "FPS",
                "FPS",
                &["AcquisitionFrameRate", "AcquisitionFrameRateAbs"],
            ),
            ("Width", "Chiều rộng", &["Width"]),
            ("Height", "Chiều cao", &["Height"]),
            ("Offset X", "Dịch X", &["OffsetX"]),
            ("Offset Y", "Dịch Y", &["OffsetY"]),
            ("Pixel Format", "Định dạng ảnh", &["PixelFormat"]),
            ("Flip X", "Lật ngang", &["ReverseX"]),
            ("Flip Y", "Lật dọc", &["ReverseY"]),
        ];
        let (Some(device), Some(tree)) = (self.device.as_ref(), self.feature_tree.as_ref()) else {
            self.ui.quick_features(Vec::new(), false, false);
            self.ui.roi(self.roi_ratio_index, false, String::new());
            return;
        };
        let mut rows = Vec::new();
        let mut flip_x = None;
        let mut flip_y = None;
        for (label, label_vi, aliases) in GROUPS {
            let detail = aliases.iter().find_map(|name| {
                let key = tree
                    .nodes
                    .iter()
                    .find(|node| node.key.map == MapKind::Device && node.key.name == *name)?
                    .key
                    .clone();
                genicam::detail(device, self.stream.as_ref(), &key)
                    .ok()
                    .filter(|detail| detail.value != "—")
            });
            let Some(detail) = detail else { continue };
            if *label == "Flip X" {
                flip_x = Some((detail.editable, detail.value == "True"));
            } else if *label == "Flip Y" {
                flip_y = Some((detail.editable, detail.value == "True"));
            }
            rows.push(QuickFeatureRow {
                id: detail.key.id().into(),
                label: (*label).into(),
                label_vi: (*label_vi).into(),
                value: detail.value.into(),
                editable: detail.editable,
            });
        }
        let rotate_available = flip_x.is_some_and(|(editable, _)| editable)
            && flip_y.is_some_and(|(editable, _)| editable);
        let rotated = flip_x.is_some_and(|(_, enabled)| enabled)
            && flip_y.is_some_and(|(_, enabled)| enabled);
        self.ui.quick_features(rows, rotate_available, rotated);
        self.show_roi();
    }

    fn selected_ratio(&self) -> Option<Ratio> {
        usize::try_from(self.roi_ratio_index - 1)
            .ok()
            .and_then(|index| roi::PRESETS.get(index).copied())
    }

    fn show_roi(&self) {
        let result = self
            .device
            .as_ref()
            .and_then(|device| genicam::roi_info(device).ok());
        // Choosing a ratio only changes application state. A camera may report
        // ROI nodes read-only during acquisition even though they become
        // writable once the worker stops the stream for an actual save.
        let available = result.is_some();
        let text = match (result, self.selected_ratio()) {
            (Some(info), Some(ratio)) => roi::largest(ratio, info.width, info.height)
                .map(|(width, height)| format!("{width} × {height}"))
                .unwrap_or_default(),
            (Some(info), None) => format!("{} × {}", info.state.width, info.state.height),
            _ => String::new(),
        };
        self.ui.roi(self.roi_ratio_index, available, text);
    }

    fn set_roi_ratio(&mut self, index: i32) -> Result<(), String> {
        if self.device.is_none() {
            return Err("Connect a camera first".to_owned());
        }
        if index != 0
            && usize::try_from(index - 1)
                .ok()
                .and_then(|i| roi::PRESETS.get(i))
                .is_none()
        {
            return Err("Unknown aspect ratio".to_owned());
        }
        self.roi_ratio_index = index;
        self.show_roi();
        Ok(())
    }

    fn show_feature_rows(&self) {
        let query = self.feature_search.trim().to_lowercase();
        let rows = self
            .feature_tree
            .as_ref()
            .map(|tree| {
                tree.nodes
                    .iter()
                    .filter(|node| {
                        if !query.is_empty() {
                            node.key.name.to_lowercase().contains(&query)
                                || node.display.to_lowercase().contains(&query)
                                || node
                                    .ancestors
                                    .iter()
                                    .any(|ancestor| ancestor.to_lowercase().contains(&query))
                        } else {
                            !node
                                .ancestors
                                .iter()
                                .any(|ancestor| self.collapsed_categories.contains(ancestor))
                        }
                    })
                    .map(|node| {
                        let marker = if node.category {
                            if self.collapsed_categories.contains(&node.key.id()) {
                                "▸"
                            } else {
                                "▾"
                            }
                        } else {
                            " "
                        };
                        let access = match node.access.as_str() {
                            "Read/write" => "",
                            "Unavailable" => " [NA]",
                            "Not implemented" => " [NI]",
                            "Write only" => " [WO]",
                            "Read only" => " [RO]",
                            _ => " [?]",
                        };
                        let hidden = if node.visibility == "Invisible" {
                            " [hidden]"
                        } else {
                            ""
                        };
                        FeatureRow {
                            id: node.key.id().into(),
                            label: format!(
                                "{}{} {}{}{}",
                                "  ".repeat(node.depth.min(7)),
                                marker,
                                node.display,
                                access,
                                hidden
                            )
                            .into(),
                            category: node.category,
                        }
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        self.ui.features(rows);
    }

    fn select_feature(&mut self, id: &str) {
        self.selected_feature = self.feature_tree.as_ref().and_then(|tree| {
            tree.nodes
                .iter()
                .find(|node| node.key.id() == id)
                .map(|node| node.key.clone())
        });
        self.show_selected_detail();
    }

    fn show_selected_detail(&self) {
        let (Some(device), Some(key)) = (self.device.as_ref(), self.selected_feature.as_ref())
        else {
            self.ui.feature_detail(None);
            return;
        };
        match genicam::detail(device, self.stream.as_ref(), key) {
            Ok(detail) => {
                let current = detail.value.clone();
                self.ui.feature_detail(Some(detail));
                self.preview_roi_value(&current);
            }
            Err(error) => {
                self.ui.feature_detail(None);
                self.ui.feature_error(error);
            }
        }
    }

    fn start_live(&mut self) -> Result<(), String> {
        if self.stream.is_some() {
            return Err("Live view is already running".to_owned());
        }
        let device = self
            .device
            .as_ref()
            .ok_or_else(|| "Connect a camera first".to_owned())?;
        self.stream = Some(Stream::start(device)?);
        self.last_frame_at = None;
        self.last_preview_at = None;
        self.last_received_at = None;
        self.live_started_at = Some(Instant::now());
        self.no_frame_warned = false;
        self.fps_since = Some(Instant::now());
        self.fps_count = 0;
        self.actual_fps = "0.0".to_owned();
        self.ui
            .fps(self.actual_fps.clone(), self.configured_fps.clone());
        self.refresh_features();
        self.log("INFO", "Live acquisition started".to_owned());
        Ok(())
    }

    fn update_fps(&mut self) {
        let (Some(since), Some(stream)) = (self.fps_since, self.stream.as_ref()) else {
            return;
        };
        let elapsed = since.elapsed();
        if elapsed < Duration::from_secs(1) {
            return;
        }
        let count = stream.received_frames();
        let frames = count.saturating_sub(self.fps_count);
        self.actual_fps = format!("{:.1}", frames as f64 / elapsed.as_secs_f64());
        self.ui
            .fps(self.actual_fps.clone(), self.configured_fps.clone());
        self.fps_count = count;
        self.fps_since = Some(Instant::now());
    }

    fn apply_roi_target(&mut self, target: genicam::RoiState, center: bool) -> Result<(), String> {
        let was_live = self.stream.is_some();
        if was_live {
            self.stop_live();
        }
        let result = (|| {
            let device = self.device.as_ref().ok_or("Connect a camera first")?;
            let before = genicam::roi_info(device)?.state;
            // Keep the first snapshot even if a camera rejects a later write and
            // rollback fails. A normal disconnect can retry restoration.
            if self.original_roi.is_none() {
                self.original_roi = Some(before);
            }
            genicam::apply_roi(device, target, center)
        })();
        let (before, after) = match result {
            Ok(change) => change,
            Err(error) => {
                let restart = if was_live {
                    self.start_live().err()
                } else {
                    None
                };
                self.refresh_features();
                return Err(format!(
                    "{error}{}",
                    restart
                        .map(|failure| format!("; live restart failed: {failure}"))
                        .unwrap_or_default()
                ));
            }
        };
        if was_live && let Err(error) = self.start_live() {
            let rollback = self
                .device
                .as_ref()
                .ok_or_else(|| "Camera disconnected before ROI rollback".to_owned())
                .and_then(|device| genicam::restore_roi(device, before))
                .err();
            let restart = if rollback.is_none() {
                self.start_live().err()
            } else {
                None
            };
            self.refresh_features();
            return Err(format!(
                "Live restart after ROI change failed: {error}{}{}",
                rollback
                    .map(|failure| format!("; rollback failed: {failure}"))
                    .unwrap_or_default(),
                restart
                    .map(|failure| format!("; restart after rollback failed: {failure}"))
                    .unwrap_or_default()
            ));
        }
        self.log("INFO", format!("Camera ROI: {before:?} -> {after:?}"));
        self.refresh_features();
        self.status("Camera ROI applied", "Đã áp dụng vùng ảnh camera");
        self.ui.close_quick_editor();
        Ok(())
    }

    fn apply_roi_feature(&mut self, key: &FeatureKey, proposed: &str) -> Result<(), String> {
        let value: i64 = proposed
            .trim()
            .parse()
            .map_err(|_| "Enter an integer ROI value")?;
        let info = genicam::roi_info(self.device.as_ref().ok_or("Connect a camera first")?)?;
        let mut target = info.state;
        let mut center = false;
        match key.name.as_str() {
            "Width" | "Height" => {
                if let Some(ratio) = self.selected_ratio() {
                    let pair = roi::paired(ratio, key.name == "Width", value, info.width, info.height)
                        .ok_or("This value cannot make an exact locked ratio within the camera limits and steps. Choose another value or switch to Free.")?;
                    target.width = pair.0;
                    target.height = pair.1;
                    center = true;
                } else if key.name == "Width" {
                    target.width = value;
                } else {
                    target.height = value;
                }
            }
            "OffsetX" => target.offset_x = value,
            "OffsetY" => target.offset_y = value,
            _ => return Err("Unsupported ROI feature".to_owned()),
        }
        self.apply_roi_target(target, center)
    }

    fn preview_roi_value(&self, proposed: &str) {
        let Some(key) = self.selected_feature.as_ref() else {
            self.ui.roi_preview(true, String::new(), String::new());
            return;
        };
        if key.map != MapKind::Device || !matches!(key.name.as_str(), "Width" | "Height") {
            self.ui.roi_preview(true, String::new(), String::new());
            return;
        }
        let Some(ratio) = self.selected_ratio() else {
            self.ui.roi_preview(true, String::new(), String::new());
            return;
        };
        let pair = proposed.trim().parse::<i64>().ok().and_then(|value| {
            let device = self.device.as_ref()?;
            let info = genicam::roi_info(device).ok()?;
            roi::paired(ratio, key.name == "Width", value, info.width, info.height)
        });
        if let Some((width, height)) = pair {
            self.ui.roi_preview(
                true,
                format!("Locked ROI: {width} × {height} px"),
                format!("ROI cố định: {width} × {height} px"),
            );
        } else {
            self.ui.roi_preview(
                false,
                "No exact paired size. Choose another value or select Free.".to_owned(),
                "Không có cặp kích thước đúng tỉ lệ. Chọn giá trị khác hoặc Tự do.".to_owned(),
            );
        }
    }

    fn apply_max_roi(&mut self) -> Result<(), String> {
        let ratio = self.selected_ratio().ok_or("Select a fixed ratio first")?;
        let info = genicam::roi_info(self.device.as_ref().ok_or("Connect a camera first")?)?;
        let (width, height) = roi::largest(ratio, info.width, info.height)
            .ok_or("Camera has no exact size for the selected ratio")?;
        self.apply_roi_target(
            genicam::RoiState {
                width,
                height,
                ..info.state
            },
            true,
        )
    }

    fn apply_feature(&mut self, proposed: &str) -> Result<(), String> {
        let key = self
            .selected_feature
            .clone()
            .ok_or_else(|| "Select a GenICam node".to_owned())?;
        if key.map == MapKind::Device
            && matches!(
                key.name.as_str(),
                "Width" | "Height" | "OffsetX" | "OffsetY"
            )
        {
            return self.apply_roi_feature(&key, proposed);
        }
        let was_live = self.stream.is_some();
        if was_live {
            self.stop_live();
        }
        let result = {
            let device = self
                .device
                .as_ref()
                .ok_or_else(|| "Connect a camera first".to_owned())?;
            genicam::apply(device, &key, proposed)
        };
        let (before, after) = match result {
            Ok(change) => change,
            Err(error) => {
                let mut message = format!("{}: {error}", key.id());
                if was_live && let Err(restart_error) = self.start_live() {
                    message.push_str(&format!("; live view could not resume: {restart_error}"));
                }
                self.show_selected_detail();
                return Err(message);
            }
        };
        if was_live && let Err(restart_error) = self.start_live() {
            let mut recovery_error = None;
            if let Some(device) = self.device.as_ref()
                && let Err(restore_error) = genicam::restore(device, &key, &before)
            {
                recovery_error = Some(restore_error);
            }
            if recovery_error.is_some()
                && !self
                    .original_features
                    .iter()
                    .any(|(saved, _)| saved == &key)
            {
                self.original_features.push((key.clone(), before.clone()));
            }
            let resumed = if recovery_error.is_none() {
                self.start_live().err()
            } else {
                None
            };
            self.refresh_features();
            return Err(format!(
                "Live view could not resume after changing {}: {restart_error}{}{}",
                key.name,
                recovery_error
                    .map(|error| format!("; rollback failed: {error}"))
                    .unwrap_or_default(),
                resumed
                    .map(|error| format!("; restart after rollback failed: {error}"))
                    .unwrap_or_default()
            ));
        }
        if !self
            .original_features
            .iter()
            .any(|(saved, _)| saved == &key)
        {
            self.original_features.push((key.clone(), before.clone()));
        }
        self.log(
            "INFO",
            format!("GenICam {}: {} -> {}", key.id(), before, after),
        );
        self.refresh_features();
        self.status("Camera setting applied", "Đã áp dụng thông số camera");
        self.ui.close_quick_editor();
        Ok(())
    }

    fn rotate_180(&mut self, enabled: bool) -> Result<(), String> {
        let keys = [
            FeatureKey {
                map: MapKind::Device,
                name: "ReverseX".to_owned(),
            },
            FeatureKey {
                map: MapKind::Device,
                name: "ReverseY".to_owned(),
            },
        ];
        let was_live = self.stream.is_some();
        if was_live {
            self.stop_live();
        }
        let target = if enabled { "True" } else { "False" };
        let mut originals = Vec::new();
        let mut write_attempted = false;
        let result = (|| -> Result<(), String> {
            let device = self.device.as_ref().ok_or("Connect a camera first")?;
            for key in &keys {
                let detail = genicam::detail(device, None, key)?;
                if !detail.editable {
                    return Err(format!("{} is not writable on this camera", key.name));
                }
                originals.push((key.clone(), detail.value));
            }
            for (key, before) in &originals {
                if before != target {
                    write_attempted = true;
                    genicam::apply(device, key, target)?;
                }
            }
            Ok(())
        })();

        if let Err(error) = result {
            let rollback = if write_attempted {
                self.restore_orientation(&originals)
            } else {
                Vec::new()
            };
            let restart = if was_live {
                self.start_live().err()
            } else {
                None
            };
            self.refresh_features();
            return Err(format!(
                "Rotation failed: {error}{}{}",
                if rollback.is_empty() {
                    String::new()
                } else {
                    format!("; rollback: {}", rollback.join("; "))
                },
                restart
                    .map(|error| format!("; live view could not resume: {error}"))
                    .unwrap_or_default()
            ));
        }

        if was_live && let Err(error) = self.start_live() {
            let rollback = self.restore_orientation(&originals);
            let restart = if rollback.is_empty() {
                self.start_live().err()
            } else {
                None
            };
            self.refresh_features();
            return Err(format!(
                "Live view could not resume after rotation: {error}{}{}",
                if rollback.is_empty() {
                    String::new()
                } else {
                    format!("; rollback: {}", rollback.join("; "))
                },
                restart
                    .map(|error| format!("; restart after rollback failed: {error}"))
                    .unwrap_or_default()
            ));
        }

        for (key, before) in originals {
            if before != target {
                if !self
                    .original_features
                    .iter()
                    .any(|(saved, _)| saved == &key)
                {
                    self.original_features.push((key.clone(), before.clone()));
                }
                self.log(
                    "INFO",
                    format!("GenICam {}: {} -> {}", key.id(), before, target),
                );
            }
        }
        self.refresh_features();
        self.status(
            if enabled {
                "Camera rotated 180°"
            } else {
                "Camera rotation cleared"
            },
            if enabled {
                "Đã xoay camera 180°"
            } else {
                "Đã bỏ xoay 180°"
            },
        );
        Ok(())
    }

    fn restore_orientation(&mut self, originals: &[(FeatureKey, String)]) -> Vec<String> {
        let mut failures = Vec::new();
        let Some(device) = self.device.as_ref() else {
            return vec!["Camera disconnected before rotation rollback".to_owned()];
        };
        for (key, original) in originals.iter().rev() {
            if let Err(error) = genicam::restore(device, key, original) {
                failures.push(format!("{}: {error}", key.id()));
                if !self.original_features.iter().any(|(saved, _)| saved == key) {
                    self.original_features.push((key.clone(), original.clone()));
                }
            }
        }
        failures
    }

    fn refresh(&mut self) {
        if self.device.is_some() {
            self.failure("Disconnect before refreshing the camera list".to_owned());
            return;
        }
        let result = self.ensure_runtime().and_then(Runtime::enumerate_with_gige);
        match result {
            Ok((cameras, warning)) => {
                let count = cameras.len();
                let unavailable = cameras.iter().filter(|camera| !camera.reachable).count();
                let previous = self.cameras.get(self.selected_camera_index);
                let selected = previous
                    .and_then(|old| {
                        cameras.iter().position(|new| match (&old.gige, &new.gige) {
                            (Some(a), Some(b)) => a.mac == b.mac,
                            _ => old.key == new.key,
                        })
                    })
                    .unwrap_or(0);
                self.selected_camera_index = selected;
                self.ui.cameras(
                    cameras.iter().map(|camera| camera.label.clone()).collect(),
                    selected,
                );
                self.ui.network_camera(cameras.get(selected));
                self.cameras = cameras;
                self.log("INFO", format!("Discovered {count} pylon camera(s), {unavailable} unavailable through normal enumeration"));
                if let Some(warning) = warning {
                    self.log("WARN", warning);
                }
                if count == 0 {
                    self.status(
                        "No camera found. Check the pylon driver and connection.",
                        "Không tìm thấy camera. Kiểm tra driver pylon và kết nối.",
                    );
                } else {
                    self.status(
                        format!("Found {count} camera(s); {unavailable} require IP/network setup"),
                        format!("Tìm thấy {count} camera; {unavailable} cần thiết lập IP/mạng"),
                    );
                }
            }
            Err(error) => self.failure(error),
        }
    }

    fn update_image_transform(&mut self, transform: ImageTransform) {
        if self.image_transform == transform {
            return;
        }
        self.image_transform = transform;
        self.ui.image_transform(transform);
        self.log(
            "INFO",
            format!(
                "App image transform: {}° clockwise, horizontal flip {}, vertical flip {}",
                transform.quarter_turns as u16 * 90,
                transform.flip_horizontal,
                transform.flip_vertical
            ),
        );
        if let Some(frame) = &self.last_frame {
            self.ui.refresh_preview(frame, transform);
        }
    }

    fn apply_model_input_size(&mut self, size: InputSize) {
        if self.model_size_source == SizeSource::Fixed {
            self.ui.model_size_error(
                "This ONNX model has a fixed input size",
                "Model ONNX này có kích thước đầu vào cố định",
            );
            return;
        }
        if size == self.model_input_size {
            self.ui
                .model_size(size, self.model_default_size, self.model_size_source);
            return;
        }
        let Some(inference) = &self.inference else {
            return;
        };
        self.live_epoch = inference.set_input_size(size);
        self.ui.model_clear_result();
        self.ui.model_status(
            true,
            true,
            "Applying inference size…".to_owned(),
            "Đang áp dụng kích thước AI…".to_owned(),
            String::new(),
        );
    }

    fn handle(&mut self, command: Command) {
        match command {
            Command::Refresh => self.refresh(),
            Command::SelectCamera(index) => {
                self.selected_camera_index = index;
                self.ui.network_camera(self.cameras.get(index));
            }
            Command::ConfigureIp {
                index,
                ip,
                mask,
                gateway,
                persistent,
            } => {
                if self.device.is_some() {
                    self.ui
                        .ip_error("Disconnect camera before changing IP".to_owned());
                    return;
                }
                let Some(camera) = self.cameras.get(index).cloned() else {
                    self.ui
                        .ip_error("Select a camera from the current list".to_owned());
                    return;
                };
                let Some(gige) = &camera.gige else {
                    self.ui
                        .ip_error("Only GigE cameras support IP configuration".to_owned());
                    return;
                };
                let settings = match IpSettings::parse(&ip, &mask, &gateway, &gige.interface_ip) {
                    Ok(settings) => settings,
                    Err(error) => {
                        self.ui.ip_error_localized(error.english, error.vietnamese);
                        return;
                    }
                };
                if self.cameras.iter().any(|other| {
                    other.gige.as_ref().is_some_and(|item| {
                        item.mac != gige.mac && item.ip == settings.ip.to_string()
                    })
                }) {
                    self.ui.ip_error_localized(
                        "Another discovered camera already uses this IP",
                        "IP này đã được một camera khác trong danh sách sử dụng",
                    );
                    return;
                }
                let mode = if persistent {
                    "persistent"
                } else {
                    "temporary"
                };
                self.log(
                    "INFO",
                    format!(
                        "Request {mode} IP for camera MAC {}: {} / {}, gateway {}",
                        gige.mac, settings.ip, settings.mask, settings.gateway
                    ),
                );
                let result = self.ensure_runtime().and_then(|runtime| {
                    runtime.configure_gige_ip(
                        &gige.mac,
                        &settings.ip.to_string(),
                        &settings.mask.to_string(),
                        &settings.gateway.to_string(),
                        &gige.user_name,
                        persistent,
                    )
                });
                match result {
                    Ok(()) => {
                        self.log(
                            "INFO",
                            format!("IP command accepted by pylon for MAC {}", gige.mac),
                        );
                        self.ui.ip_applied();
                        for _ in 0..6 {
                            let found = self
                                .ensure_runtime()
                                .and_then(Runtime::enumerate_with_gige)
                                .ok()
                                .is_some_and(|(cameras, _)| {
                                    cameras.iter().any(|item| {
                                        item.gige.as_ref().is_some_and(|network| {
                                            network.mac.eq_ignore_ascii_case(&gige.mac)
                                                && network.ip == settings.ip.to_string()
                                        })
                                    })
                                });
                            if found {
                                break;
                            }
                            std::thread::sleep(Duration::from_millis(250));
                        }
                        self.refresh();
                        let confirmed = self.cameras.iter().any(|item| {
                            item.gige.as_ref().is_some_and(|network| {
                                network.mac.eq_ignore_ascii_case(&gige.mac)
                                    && network.ip == settings.ip.to_string()
                            })
                        });
                        if confirmed {
                            self.log(
                                "INFO",
                                format!("IP read back for MAC {}: {}", gige.mac, settings.ip),
                            );
                            if persistent {
                                self.status(
                                    format!("Static IP saved; camera found at {}", settings.ip),
                                    format!(
                                        "Đã lưu IP cố định; tìm lại camera tại {}",
                                        settings.ip
                                    ),
                                );
                            } else {
                                self.status(
                                    format!("Temporary camera IP applied: {}", settings.ip),
                                    format!("Đã áp dụng IP tạm thời: {}", settings.ip),
                                );
                            }
                        } else {
                            self.log("WARN", format!("IP command sent for MAC {}, but target IP {} was not found on rescan", gige.mac, settings.ip));
                            self.status(
                                "IP command sent; camera not confirmed on rescan. Check network and log.",
                                "Đã gửi lệnh đổi IP nhưng chưa tìm lại được camera. Kiểm tra mạng và log.",
                            );
                        }
                    }
                    Err(error) => {
                        self.log(
                            "ERROR",
                            format!("IP configuration failed for MAC {}: {error}", gige.mac),
                        );
                        self.ui.ip_error(error);
                    }
                }
            }
            Command::Connect(index) => {
                if self.device.is_some() {
                    self.failure("A camera is already connected".to_owned());
                    return;
                }
                let Some(camera) = self.cameras.get(index).cloned() else {
                    self.failure("Select a camera from the current list".to_owned());
                    return;
                };
                if !camera.reachable {
                    self.failure(
                        "Camera is on a different subnet. Configure its IP first.".to_owned(),
                    );
                    return;
                }
                let result = self
                    .ensure_runtime()
                    .and_then(|runtime| runtime.connect(&camera.key))
                    .and_then(|device| Converter::new().map(|converter| (device, converter)));
                match result {
                    Ok((device, converter)) => {
                        self.device = Some(device);
                        self.converter = Some(converter);
                        self.ui.clear_preview();
                        if let Some(device) = self.device.as_ref() {
                            match genicam::device_properties(device) {
                                Ok(properties) => self.ui.device_info(
                                    properties
                                        .into_iter()
                                        .map(|(name, value)| format!("{name}: {value}"))
                                        .collect::<Vec<_>>()
                                        .join("\n"),
                                ),
                                Err(error) => {
                                    self.log("WARN", format!("Device information: {error}"))
                                }
                            }
                        }
                        self.refresh_features();
                        self.log("INFO", format!("Connected: {}", camera.label));
                        self.status("Camera connected", "Đã kết nối camera");
                    }
                    Err(error) => self.failure(error),
                }
            }
            Command::Disconnect => {
                let failures = self.disconnect();
                if failures.is_empty() {
                    self.status("Camera disconnected", "Đã ngắt kết nối camera");
                } else {
                    self.status(
                        "Camera disconnected; some settings could not be restored. See log.",
                        "Đã ngắt kết nối; một số thông số không khôi phục được. Xem log.",
                    );
                }
            }
            Command::StartLive => match self.start_live() {
                Ok(()) => self.status("Live view running", "Đang xem trực tiếp"),
                Err(error) => self.failure(error),
            },
            Command::StopLive => {
                self.stop_live();
                self.refresh_features();
                self.status("Live view stopped", "Đã dừng live view");
            }
            Command::Capture => match self.capture() {
                Ok(path) => {
                    self.log("INFO", format!("PNG saved: {path}"));
                    self.status("Image captured and saved", "Đã chụp và lưu ảnh");
                }
                Err(error) => self.failure(error),
            },
            Command::RefreshFeatures => {
                if self.device.is_none() {
                    self.failure("Connect a camera first".to_owned());
                } else {
                    self.refresh_features();
                    self.status("Camera features refreshed", "Đã làm mới thông số camera");
                }
            }
            Command::SearchFeatures(query) => {
                self.feature_search = query;
                self.show_feature_rows();
            }
            Command::SelectFeature(id) => self.select_feature(&id),
            Command::ToggleCategory(id) => {
                if !self.collapsed_categories.insert(id.clone()) {
                    self.collapsed_categories.remove(&id);
                }
                self.show_feature_rows();
            }
            Command::ApplyFeature(value) => {
                if let Err(error) = self.apply_feature(&value) {
                    self.ui.feature_error(error.clone());
                    self.failure(error);
                }
            }
            Command::PreviewRoiValue(value) => {
                self.preview_roi_value(&value);
            }
            Command::SetRoiRatio(index) => {
                if let Err(error) = self.set_roi_ratio(index) {
                    self.failure(error);
                }
            }
            Command::ApplyMaxRoi => {
                if let Err(error) = self.apply_max_roi() {
                    self.ui.feature_error(error.clone());
                    self.failure(error);
                }
            }
            Command::Rotate180(enabled) => {
                if let Err(error) = self.rotate_180(enabled) {
                    self.ui.feature_error(error.clone());
                    self.failure(error);
                }
            }
            Command::RotateImage90 => {
                let mut transform = self.image_transform;
                transform.quarter_turns = (transform.quarter_turns + 1) % 4;
                self.update_image_transform(transform);
            }
            Command::ToggleImageHorizontal => {
                let mut transform = self.image_transform;
                transform.flip_horizontal = !transform.flip_horizontal;
                self.update_image_transform(transform);
            }
            Command::ToggleImageVertical => {
                let mut transform = self.image_transform;
                transform.flip_vertical = !transform.flip_vertical;
                self.update_image_transform(transform);
            }
            Command::ResetImageTransform => {
                self.update_image_transform(ImageTransform::default());
            }
            Command::ExportModel(path) => {
                if self.exporting_model {
                    return;
                }
                self.exporting_model = true;
                self.model_ready = false;
                self.model_looping = false;
                self.ui.model_looping(false);
                if let Some(inference) = &self.inference {
                    self.model_generation = inference.unload();
                }
                self.ui.model_clear_result();
                self.ui.model_clear_path();
                self.ui.model_size_reset();
                self.ui.model_status(
                    false,
                    true,
                    "Exporting ONNX…".to_owned(),
                    "Đang xuất ONNX…".to_owned(),
                    String::new(),
                );
                let sender = self.sender.clone();
                let spawned = std::thread::Builder::new()
                    .name("onnx-export".to_owned())
                    .spawn(move || {
                        let result = model_export::export(&path);
                        let _ = sender.send(Command::ModelExported(result));
                    });
                if let Err(error) = spawned {
                    self.exporting_model = false;
                    let message = format!("Could not start ONNX export: {error}");
                    self.ui
                        .model_status(false, false, message.clone(), message, String::new());
                }
            }
            Command::ModelExported(result) => {
                self.exporting_model = false;
                match result {
                    Ok(path) => {
                        self.log("INFO", format!("ONNX model exported: {}", path.display()));
                        self.handle(Command::LoadModel(path));
                    }
                    Err(error) => {
                        self.log("ERROR", error.clone());
                        self.ui
                            .model_status(false, false, error.clone(), error, String::new());
                    }
                }
            }
            Command::LoadModel(path) => {
                self.model_ready = false;
                self.model_looping = false;
                self.ui.model_looping(false);
                self.ui.model_clear_result();
                self.ui.model_size_reset();
                self.ui.model_status(
                    false,
                    true,
                    "Loading ONNX model…".to_owned(),
                    "Đang mở model ONNX…".to_owned(),
                    path.display().to_string(),
                );
                if let Some(inference) = &self.inference {
                    self.model_generation = inference.load(path);
                } else {
                    self.ui.model_status(
                        false,
                        false,
                        "Inference worker is unavailable".to_owned(),
                        "Luồng AI không khả dụng".to_owned(),
                        String::new(),
                    );
                }
            }
            Command::UnloadModel => {
                self.model_ready = false;
                self.model_looping = false;
                self.ui.model_looping(false);
                self.ui.model_clear_result();
                self.ui.model_clear_path();
                self.ui.model_size_reset();
                self.ui.model_status(
                    false,
                    false,
                    "Model stopped".to_owned(),
                    "Đã dừng AI".to_owned(),
                    String::new(),
                );
                if let Some(inference) = &self.inference {
                    self.model_generation = inference.unload();
                }
            }
            Command::SetModelSource(index) => {
                let source = if index == 1 {
                    ModelSource::Image
                } else {
                    ModelSource::Live
                };
                if self.model_source == source {
                    return;
                }
                let was_looping = self.model_looping;
                self.model_looping = false;
                self.ui.model_looping(false);
                self.model_source = source;
                if let Some(inference) = &self.inference {
                    self.live_epoch = if was_looping {
                        inference.stop_loop()
                    } else {
                        inference.clear_frames()
                    };
                }
                self.ui.model_clear_result();
                self.ui.model_source(
                    source,
                    self.model_input
                        .as_ref()
                        .map(|input| input.path().display().to_string())
                        .unwrap_or_default(),
                );
                if source == ModelSource::Image {
                    if let Some(LoopInput::Image(path)) =
                        self.model_input.clone().filter(|_| self.model_ready)
                    {
                        self.handle(Command::TestModelImage(path));
                    } else if self.model_ready {
                        let folder_selected =
                            matches!(self.model_input.as_ref(), Some(LoopInput::Folder(_)));
                        self.ui.model_status(
                            true,
                            false,
                            if folder_selected {
                                "Folder selected; start loop"
                            } else {
                                "Choose an image to test"
                            }
                            .to_owned(),
                            if folder_selected {
                                "Đã chọn thư mục; bấm Chạy lặp"
                            } else {
                                "Chọn ảnh để thử model"
                            }
                            .to_owned(),
                            String::new(),
                        );
                    }
                } else if self.model_ready {
                    self.ui.model_status(
                        true,
                        false,
                        if self.stream.is_some() {
                            "Model running"
                        } else {
                            "Model ready; start live view"
                        }
                        .to_owned(),
                        if self.stream.is_some() {
                            "AI đang xử lý"
                        } else {
                            "Model sẵn sàng; bật live view"
                        }
                        .to_owned(),
                        String::new(),
                    );
                }
            }
            Command::TestModelImage(path) => {
                if !self.model_ready {
                    self.ui.model_status(
                        false,
                        false,
                        "Open an ONNX model first".to_owned(),
                        "Mở model ONNX trước".to_owned(),
                        String::new(),
                    );
                    return;
                }
                self.model_source = ModelSource::Image;
                self.model_input = Some(LoopInput::Image(path.clone()));
                self.model_looping = false;
                self.ui.model_looping(false);
                self.ui
                    .model_source(ModelSource::Image, path.display().to_string());
                self.ui.model_clear_result();
                self.ui.model_status(
                    true,
                    true,
                    "Processing test image...".to_owned(),
                    "Đang xử lý ảnh thử...".to_owned(),
                    String::new(),
                );
                if let Some(inference) = &self.inference {
                    self.live_epoch = inference.test_image(path);
                }
            }
            Command::SelectModelFolder(path) => {
                if !self.model_ready {
                    return;
                }
                if !path.is_dir() {
                    self.ui.model_image_error(
                        "Selected folder is unavailable",
                        "Không mở được thư mục đã chọn",
                    );
                    return;
                }
                self.model_source = ModelSource::Image;
                self.model_input = Some(LoopInput::Folder(path.clone()));
                self.model_looping = false;
                self.ui.model_looping(false);
                if let Some(inference) = &self.inference {
                    self.live_epoch = inference.stop_loop();
                }
                self.ui.model_clear_result();
                self.ui
                    .model_source(ModelSource::Image, path.display().to_string());
                self.ui.model_status(
                    true,
                    false,
                    "Folder selected; start loop".to_owned(),
                    "Đã chọn thư mục; bấm Chạy lặp".to_owned(),
                    String::new(),
                );
            }
            Command::StartModelLoop => {
                let Some(input) = self.model_input.clone().filter(|_| self.model_ready) else {
                    return;
                };
                if self.model_looping {
                    return;
                }
                self.model_source = ModelSource::Image;
                self.model_looping = true;
                self.ui.model_looping(true);
                self.ui
                    .model_source(ModelSource::Image, input.path().display().to_string());
                self.ui.model_clear_result();
                self.ui.model_status(
                    true,
                    false,
                    "Image loop running".to_owned(),
                    "Đang detect ảnh liên tục".to_owned(),
                    String::new(),
                );
                if let Some(inference) = &self.inference {
                    self.live_epoch = inference.start_loop(input);
                }
            }
            Command::StopModelLoop => {
                if !self.model_looping {
                    return;
                }
                self.model_looping = false;
                self.ui.model_looping(false);
                if let Some(inference) = &self.inference {
                    self.live_epoch = inference.stop_loop();
                }
                self.ui.model_status(
                    true,
                    false,
                    "Image loop stopped".to_owned(),
                    "Đã dừng detect liên tục".to_owned(),
                    String::new(),
                );
            }
            Command::SetModelInputSize(value) => {
                if !self.model_ready {
                    return;
                }
                let Some(size) = InputSize::from_user(&value) else {
                    self.ui.model_size_error(
                        "Multiple of 32, 64–2048 (e.g. 704)",
                        "Bội số 32, từ 64–2048 (vd: 704)",
                    );
                    return;
                };
                self.apply_model_input_size(size);
            }
            Command::ResetModelInputSize => {
                if self.model_ready {
                    self.apply_model_input_size(self.model_default_size);
                }
            }
            Command::SetAnnotationMode(index) => {
                if !self.model_ready {
                    self.ui.annotation_mode(self.annotation_mode);
                    return;
                }
                let mode = match index {
                    0 => AnnotationMode::Dot,
                    1 => AnnotationMode::Box,
                    _ => {
                        self.ui.annotation_mode(self.annotation_mode);
                        return;
                    }
                };
                if mode == self.annotation_mode {
                    self.ui.annotation_mode(mode);
                    return;
                }
                let Some(inference) = &self.inference else {
                    return;
                };
                self.live_epoch = inference.set_annotation_mode(mode);
                self.ui.model_clear_result();
                self.ui.model_status(
                    true,
                    true,
                    "Changing result display…".to_owned(),
                    "Đang đổi kiểu hiển thị…".to_owned(),
                    String::new(),
                );
            }
            Command::OpenModelDetail => self.ui.open_model_detail(),
            Command::CloseModelDetail => self.ui.close_model_detail(),
            Command::InferenceEvent(event) => match event {
                InferenceEvent::Loaded(generation, path, model_input) => {
                    if generation == self.model_generation {
                        self.model_ready = true;
                        self.model_input_size = model_input.active;
                        self.model_default_size = model_input.default;
                        self.model_size_source = model_input.source;
                        self.ui.model_size(
                            model_input.active,
                            model_input.default,
                            model_input.source,
                        );
                        self.log("INFO", format!("ONNX model loaded: {}", path.display()));
                        self.log(
                            "INFO",
                            format!(
                                "ONNX inference input: {} ({:?})",
                                model_input.active.label(),
                                model_input.source
                            ),
                        );
                        let (english, vietnamese) = if self.model_source == ModelSource::Image {
                            ("Model ready; choose an image", "Model sẵn sàng; chọn ảnh")
                        } else if self.stream.is_some() {
                            ("Model running", "AI đang xử lý")
                        } else {
                            (
                                "Model ready; start live view",
                                "Model đã sẵn sàng; bật live view",
                            )
                        };
                        self.ui.model_status(
                            true,
                            false,
                            english.to_owned(),
                            vietnamese.to_owned(),
                            path.display().to_string(),
                        );
                    }
                }
                InferenceEvent::SizeChanged(generation, epoch, size) => {
                    if generation == self.model_generation && epoch == self.live_epoch {
                        self.model_input_size = size;
                        self.ui
                            .model_size(size, self.model_default_size, self.model_size_source);
                        self.log(
                            "INFO",
                            format!("ONNX inference input changed to {}", size.label()),
                        );
                        if self.model_source == ModelSource::Image && !self.model_looping {
                            if let Some(LoopInput::Image(path)) = self.model_input.clone() {
                                self.handle(Command::TestModelImage(path));
                            }
                        } else if self.model_looping {
                            self.ui.model_status(
                                true,
                                false,
                                "Image loop running".to_owned(),
                                "Đang detect ảnh liên tục".to_owned(),
                                String::new(),
                            );
                        } else {
                            let running = self.stream.is_some();
                            self.ui.model_status(
                                true,
                                false,
                                if running {
                                    "Model running"
                                } else {
                                    "Model ready; start live view"
                                }
                                .to_owned(),
                                if running {
                                    "AI đang xử lý"
                                } else {
                                    "Model sẵn sàng; bật live view"
                                }
                                .to_owned(),
                                String::new(),
                            );
                        }
                    }
                }
                InferenceEvent::AnnotationModeChanged(generation, epoch, mode) => {
                    if generation == self.model_generation && epoch == self.live_epoch {
                        self.annotation_mode = mode;
                        self.ui.annotation_mode(mode);
                        self.log("INFO", format!("AI result display: {:?}", mode));
                        if self.model_source == ModelSource::Image && !self.model_looping {
                            if let Some(LoopInput::Image(path)) = self.model_input.clone() {
                                self.handle(Command::TestModelImage(path));
                                return;
                            }
                        }
                        let (english, vietnamese) = if self.model_looping {
                            ("Image loop running", "Đang detect ảnh liên tục")
                        } else if self.model_source == ModelSource::Image {
                            ("Model ready; choose an image", "Model sẵn sàng; chọn ảnh")
                        } else if self.stream.is_some() {
                            ("Model running", "AI đang xử lý")
                        } else {
                            (
                                "Model ready; start live view",
                                "Model sẵn sàng; bật live view",
                            )
                        };
                        self.ui.model_status(
                            true,
                            false,
                            english.to_owned(),
                            vietnamese.to_owned(),
                            String::new(),
                        );
                    }
                }
                InferenceEvent::Unloaded(generation) => {
                    if generation == self.model_generation && !self.exporting_model {
                        self.model_ready = false;
                        self.log("INFO", "ONNX model stopped".to_owned());
                        self.ui.model_status(
                            false,
                            false,
                            "Model stopped".to_owned(),
                            "Đã dừng AI".to_owned(),
                            String::new(),
                        );
                    }
                }
                InferenceEvent::Failed(generation, error) => {
                    if generation == self.model_generation {
                        self.model_ready = false;
                        self.model_looping = false;
                        self.ui.model_looping(false);
                        self.log("ERROR", format!("Model inference: {error}"));
                        self.ui.model_clear_result();
                        self.ui
                            .model_status(false, false, error.clone(), error, String::new());
                    }
                }
                InferenceEvent::ImageFailed(generation, epoch, error) => {
                    if generation == self.model_generation
                        && epoch == self.live_epoch
                        && self.model_source == ModelSource::Image
                    {
                        self.log("ERROR", error.detail());
                        self.ui.model_clear_result();
                        let (english, vietnamese) = error.ui_messages();
                        self.ui.model_image_error(english, vietnamese);
                    }
                }
                InferenceEvent::LoopSkipped(generation, epoch, path, error) => {
                    if generation == self.model_generation
                        && epoch == self.live_epoch
                        && self.model_looping
                    {
                        self.log("WARN", format!("Skipped {}: {error}", path.display()));
                    }
                }
                InferenceEvent::LoopFailed(generation, epoch, error) => {
                    if generation == self.model_generation
                        && epoch == self.live_epoch
                        && self.model_looping
                    {
                        self.model_looping = false;
                        self.ui.model_looping(false);
                        self.log("ERROR", format!("Image loop stopped: {error}"));
                        self.ui.model_image_error(
                            "No readable images; check the selection",
                            "Không có ảnh đọc được; kiểm tra lựa chọn",
                        );
                    }
                }
                InferenceEvent::Result(result) => {
                    if result.generation == self.model_generation
                        && result.frame_epoch == self.live_epoch
                        && result.source == self.model_source
                        && (result.source == ModelSource::Image || self.stream.is_some())
                    {
                        if result.source == ModelSource::Image && !result.continuous {
                            self.ui.model_status(
                                true,
                                false,
                                "Test image processed".to_owned(),
                                "Đã xử lý ảnh thử".to_owned(),
                                String::new(),
                            );
                        }
                        self.ui.model_result(result);
                    }
                }
            },
            Command::Shutdown => {}
        }
    }

    fn poll_live(&mut self) {
        let Some(stream) = self.stream.as_mut() else {
            return;
        };
        let Some(converter) = self.converter.as_ref() else {
            self.failure("Image converter is unavailable".to_owned());
            self.stop_live();
            return;
        };
        let preview_due = !self.ui.preview_pending()
            && self
                .last_preview_at
                .is_none_or(|at| at.elapsed() >= Duration::from_millis(66));
        let infer_due = self.model_ready
            && self.model_source == ModelSource::Live
            && self
                .inference
                .as_ref()
                .is_some_and(InferenceRunner::wants_live_frame);
        let convert = preview_due || infer_due;
        let before = stream.received_frames();
        let result = stream.next_frame(converter, convert);
        if stream.received_frames() > before {
            self.last_received_at = Some(Instant::now());
            if self.no_frame_warned {
                self.status("Live view running", "Đang xem trực tiếp");
            }
            self.no_frame_warned = false;
        }
        match result {
            Ok(Some(frame)) => {
                self.last_frame_at = Some(Instant::now());
                let frame = Arc::new(frame);
                if infer_due {
                    if let Some(inference) = &self.inference {
                        inference.submit(Arc::clone(&frame), self.image_transform);
                    }
                }
                if preview_due {
                    self.ui.frame(&frame, false, self.image_transform);
                    self.last_preview_at = self.last_frame_at;
                }
                self.last_frame = Some(frame);
            }
            Ok(None) => {
                if self
                    .last_received_at
                    .or(self.live_started_at)
                    .is_some_and(|at| at.elapsed() > Duration::from_secs(3))
                    && !self.no_frame_warned
                {
                    self.no_frame_warned = true;
                    self.log("WARN", "Waiting for camera frames".to_owned());
                    self.status("Waiting for camera frames", "Đang chờ ảnh từ camera");
                }
            }
            Err(error) => {
                self.failure(error);
                self.stop_live();
                self.status(
                    "Live view stopped after an error",
                    "Live view đã dừng do lỗi",
                );
            }
        }
        self.update_fps();
    }

    fn capture(&mut self) -> Result<String, String> {
        let device = self
            .device
            .as_ref()
            .ok_or_else(|| "Connect a camera first".to_owned())?;
        let converter = self
            .converter
            .as_ref()
            .ok_or_else(|| "Image converter is unavailable".to_owned())?;
        let recent = self
            .last_frame_at
            .is_some_and(|at| at.elapsed() < Duration::from_millis(250));
        let frame = if self.stream.is_some() && recent {
            Arc::clone(self.last_frame.as_ref().expect("recent frame exists"))
        } else {
            let mut local_stream = if self.stream.is_none() {
                Some(Stream::start(device)?)
            } else {
                None
            };
            let deadline = Instant::now() + Duration::from_secs(3);
            let frame = loop {
                let stream = if let Some(stream) = local_stream.as_mut() {
                    stream
                } else {
                    self.stream.as_mut().expect("live stream exists")
                };
                if let Some(frame) = stream.next_frame(converter, true)? {
                    break Arc::new(frame);
                }
                if Instant::now() >= deadline {
                    return Err("Timed out waiting for a camera frame".to_owned());
                }
            };
            if let Some(mut stream) = local_stream {
                for error in stream.stop() {
                    self.log("WARN", error);
                }
            }
            self.last_frame = Some(Arc::clone(&frame));
            self.last_frame_at = Some(Instant::now());
            frame
        };
        let directory = files::capture_dir().map_err(|error| format!("Capture folder: {error}"))?;
        let path = files::next_capture_path(&directory, &Timestamp::now())
            .map_err(|error| format!("Capture filename: {error}"))?;
        let transformed =
            (!self.image_transform.is_identity()).then(|| self.image_transform.apply(&frame));
        let output = transformed.as_ref().unwrap_or(frame.as_ref());
        output.save_png(&path)?;
        self.ui.frame(output, true, ImageTransform::default());
        let display = path.display().to_string();
        self.ui.saved(display.clone());
        Ok(display)
    }

    fn stop_live(&mut self) {
        if self.model_source == ModelSource::Live {
            if let Some(inference) = &self.inference {
                self.live_epoch = inference.clear_frames();
            }
            self.ui.model_clear_result();
        }
        if let Some(mut stream) = self.stream.take() {
            for error in stream.stop() {
                self.log("WARN", error);
            }
            self.log("INFO", "Live acquisition stopped".to_owned());
        }
        self.live_started_at = None;
        self.last_received_at = None;
        self.fps_since = None;
        self.fps_count = 0;
        self.actual_fps = "—".to_owned();
        self.ui
            .fps(self.actual_fps.clone(), self.configured_fps.clone());
    }

    fn disconnect(&mut self) -> Vec<String> {
        let mut failures = Vec::new();
        self.stop_live();
        if let Some(device) = self.device.as_ref() {
            let changes = std::mem::take(&mut self.original_features);
            for (key, original) in changes.into_iter().rev() {
                match genicam::restore(device, &key, &original) {
                    Ok(()) => self.log(
                        "INFO",
                        format!("Restored GenICam {} to {}", key.id(), original),
                    ),
                    Err(error) => {
                        self.log(
                            "ERROR",
                            format!("Could not restore GenICam {}: {error}", key.id()),
                        );
                        failures.push(format!("{}: {error}", key.id()));
                    }
                }
            }
            if let Some(original) = self.original_roi.take() {
                match genicam::restore_roi(device, original) {
                    Ok(()) => self.log("INFO", format!("Restored camera ROI to {original:?}")),
                    Err(error) => {
                        self.log("ERROR", format!("Could not restore camera ROI: {error}"));
                        failures.push(format!("ROI: {error}"));
                    }
                }
            }
        }
        self.converter.take();
        if let Some(mut device) = self.device.take() {
            for error in device.close() {
                self.log("WARN", format!("Camera cleanup: {error}"));
                failures.push(error);
            }
            self.log("INFO", "Camera disconnected".to_owned());
        }
        self.last_frame = None;
        self.last_frame_at = None;
        self.last_preview_at = None;
        self.last_received_at = None;
        self.ui.clear_preview();
        self.feature_tree = None;
        self.selected_feature = None;
        self.collapsed_categories.clear();
        self.original_features.clear();
        self.original_roi = None;
        self.roi_ratio_index = 0;
        self.configured_fps = "—".to_owned();
        self.ui
            .fps(self.actual_fps.clone(), self.configured_fps.clone());
        self.ui.features(Vec::new());
        self.ui.quick_features(Vec::new(), false, false);
        self.ui.roi(0, false, String::new());
        self.image_transform = ImageTransform::default();
        self.ui.image_transform(self.image_transform);
        self.ui.feature_detail(None);
        self.ui.device_info(String::new());
        failures
    }
}
