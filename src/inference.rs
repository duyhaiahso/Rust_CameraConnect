use crate::camera::Frame;
use crate::transform::ImageTransform;
use crate::worker::Command;
use ort::session::Session;
use ort::value::Tensor;
use std::cmp::Ordering;
use std::path::PathBuf;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

const INPUT_SIZE: u32 = 1600;
const CONFIDENCE: f32 = 0.25;
const IOU_THRESHOLD: f32 = 0.7;
const MAX_DETECTIONS: usize = 300;

#[derive(Clone, Copy)]
pub struct Detection {
    pub center_x: f32,
    pub center_y: f32,
    x1: f32,
    y1: f32,
    x2: f32,
    y2: f32,
    score: f32,
}

struct InferenceOutput {
    detections: Vec<Detection>,
    preprocess_ms: f64,
    model_ms: f64,
    decode_ms: f64,
}

pub struct ResultFrame {
    pub generation: u64,
    pub frame_epoch: u64,
    pub source: Source,
    pub preview: Frame,
    pub count: usize,
    pub model_ms: f64,
    pub processing_ms: f64,
    pub processed_fps: Option<f64>,
    pub age_ms: Option<u128>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Live,
    Image,
}

pub enum Event {
    Loaded(u64, PathBuf),
    Unloaded(u64),
    Failed(u64, String),
    ImageFailed(u64, u64, ImageFailure),
    Result(ResultFrame),
}

pub enum ImageFailure {
    Empty,
    Read(String),
    Inference(String),
}

impl ImageFailure {
    pub fn detail(&self) -> String {
        match self {
            Self::Empty => "Test image file is empty (0 bytes)".to_owned(),
            Self::Read(error) => format!("Could not read test image: {error}"),
            Self::Inference(error) => format!("Could not process test image: {error}"),
        }
    }

    pub fn ui_messages(&self) -> (&'static str, &'static str) {
        match self {
            Self::Empty => ("Empty image (0 bytes)", "Ảnh rỗng (0 byte)"),
            Self::Read(_) => (
                "Cannot read image; choose another",
                "Không đọc được ảnh; chọn ảnh khác",
            ),
            Self::Inference(_) => ("Image processing failed", "Không xử lý được ảnh"),
        }
    }
}

struct InputFrame {
    generation: u64,
    frame_epoch: u64,
    source: Source,
    frame: Arc<Frame>,
    transform: ImageTransform,
    captured_at: Instant,
}

enum Action {
    Load(u64, PathBuf),
    Unload(u64),
    TestImage(u64, u64, PathBuf),
}

#[derive(Default)]
struct State {
    generation: u64,
    frame_epoch: u64,
    action: Option<Action>,
    latest: Option<InputFrame>,
    shutdown: bool,
}

pub struct Runner {
    shared: Arc<(Mutex<State>, Condvar)>,
    thread: Option<JoinHandle<()>>,
}

impl Runner {
    pub fn new(sender: Sender<Command>) -> Result<Self, String> {
        let shared = Arc::new((Mutex::new(State::default()), Condvar::new()));
        let thread_shared = Arc::clone(&shared);
        let thread = std::thread::Builder::new()
            .name("onnx-inference".to_owned())
            .spawn(move || run(thread_shared, sender))
            .map_err(|error| format!("Could not start inference worker: {error}"))?;
        Ok(Self {
            shared,
            thread: Some(thread),
        })
    }

    pub fn load(&self, path: PathBuf) -> u64 {
        let (lock, wake) = &*self.shared;
        if let Ok(mut state) = lock.lock() {
            state.generation += 1;
            state.latest = None;
            state.action = Some(Action::Load(state.generation, path));
            wake.notify_one();
            state.generation
        } else {
            0
        }
    }

    pub fn unload(&self) -> u64 {
        let (lock, wake) = &*self.shared;
        if let Ok(mut state) = lock.lock() {
            state.generation += 1;
            state.latest = None;
            state.action = Some(Action::Unload(state.generation));
            wake.notify_one();
            state.generation
        } else {
            0
        }
    }

    pub fn clear_frames(&self) -> u64 {
        let (lock, _) = &*self.shared;
        if let Ok(mut state) = lock.lock() {
            state.frame_epoch += 1;
            state.latest = None;
            state.frame_epoch
        } else {
            0
        }
    }

    pub fn submit(&self, frame: Arc<Frame>, transform: ImageTransform) {
        let (lock, wake) = &*self.shared;
        if let Ok(mut state) = lock.lock() {
            state.latest = Some(InputFrame {
                generation: state.generation,
                frame_epoch: state.frame_epoch,
                source: Source::Live,
                frame,
                transform,
                captured_at: Instant::now(),
            });
            wake.notify_one();
        }
    }

    pub fn wants_live_frame(&self) -> bool {
        let (lock, _) = &*self.shared;
        lock.lock().is_ok_and(|state| {
            !state.shutdown
                && state.action.is_none()
                // Keep the waiting slot current without converting every camera frame.
                && state.latest.as_ref().is_none_or(|pending| {
                    pending.captured_at.elapsed() >= Duration::from_millis(33)
                })
        })
    }

    pub fn test_image(&self, path: PathBuf) -> u64 {
        let (lock, wake) = &*self.shared;
        if let Ok(mut state) = lock.lock() {
            state.frame_epoch += 1;
            state.latest = None;
            state.action = Some(Action::TestImage(state.generation, state.frame_epoch, path));
            wake.notify_one();
            state.frame_epoch
        } else {
            0
        }
    }
}

impl Drop for Runner {
    fn drop(&mut self) {
        let (lock, wake) = &*self.shared;
        if let Ok(mut state) = lock.lock() {
            state.shutdown = true;
            state.latest = None;
            wake.notify_one();
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn run(shared: Arc<(Mutex<State>, Condvar)>, sender: Sender<Command>) {
    let mut session: Option<Session> = None;
    let mut environment = None;
    let mut previous_completed: Option<Instant> = None;
    loop {
        let (action, input) = {
            let (lock, wake) = &*shared;
            let mut state = match lock.lock() {
                Ok(state) => state,
                Err(_) => return,
            };
            while !state.shutdown && state.action.is_none() && state.latest.is_none() {
                state = match wake.wait(state) {
                    Ok(state) => state,
                    Err(_) => return,
                };
            }
            if state.shutdown {
                return;
            }
            if let Some(action) = state.action.take() {
                (Some(action), None)
            } else {
                (None, state.latest.take())
            }
        };
        let input = if let Some(action) = action {
            match action {
                Action::Load(generation, path) => {
                    session = None;
                    previous_completed = None;
                    let opened = (|| -> Result<Session, String> {
                        if !path.is_file()
                            || path.extension().and_then(|x| x.to_str()) != Some("onnx")
                        {
                            return Err("Select an existing .onnx model file".to_owned());
                        }
                        if environment.is_none() {
                            environment = Some(
                                ort::init()
                                    .build()
                                    .map_err(|e| format!("ONNX Runtime: {e}"))?,
                            );
                        }
                        let env = environment.as_ref().expect("environment just initialized");
                        let loaded = Session::builder(env)
                            .and_then(|mut builder| builder.commit_from_file(&path))
                            .map_err(|e| format!("Could not open ONNX model: {e}"))?;
                        if loaded.inputs().len() != 1 || loaded.outputs().len() < 2 {
                            return Err("Expected one RGB input and two YOLO segmentation outputs"
                                .to_owned());
                        }
                        Ok(loaded)
                    })();
                    match opened {
                        Ok(loaded) => {
                            session = Some(loaded);
                            let _ = sender
                                .send(Command::InferenceEvent(Event::Loaded(generation, path)));
                        }
                        Err(error) => {
                            let _ = sender
                                .send(Command::InferenceEvent(Event::Failed(generation, error)));
                        }
                    }
                    continue;
                }
                Action::Unload(generation) => {
                    session = None;
                    previous_completed = None;
                    let _ = sender.send(Command::InferenceEvent(Event::Unloaded(generation)));
                    continue;
                }
                Action::TestImage(generation, frame_epoch, path) => {
                    previous_completed = None;
                    let opened = (|| {
                        let metadata = std::fs::metadata(&path)
                            .map_err(|error| ImageFailure::Read(error.to_string()))?;
                        if metadata.len() == 0 {
                            return Err(ImageFailure::Empty);
                        }
                        image::open(&path).map_err(|error| ImageFailure::Read(error.to_string()))
                    })();
                    match opened {
                        Ok(image) => {
                            let rgb = image.to_rgb8();
                            Some(InputFrame {
                                generation,
                                frame_epoch,
                                source: Source::Image,
                                frame: Arc::new(Frame {
                                    width: rgb.width(),
                                    height: rgb.height(),
                                    rgb: rgb.into_raw(),
                                }),
                                transform: ImageTransform::default(),
                                captured_at: Instant::now(),
                            })
                        }
                        Err(error) => {
                            let _ = sender.send(Command::InferenceEvent(Event::ImageFailed(
                                generation,
                                frame_epoch,
                                error,
                            )));
                            None
                        }
                    }
                }
            }
        } else {
            input
        };
        let (Some(input), Some(model)) = (input, session.as_mut()) else {
            continue;
        };
        let started = Instant::now();
        match infer(model, &input.frame) {
            Ok(output) => {
                let count = output.detections.len();
                let annotated = annotate(&input.frame, &output.detections);
                let preview = preview(&annotated, input.transform);
                let completed = Instant::now();
                let processed_fps = if input.source == Source::Live {
                    let fps = previous_completed
                        .map(|previous| 1.0 / completed.duration_since(previous).as_secs_f64());
                    previous_completed = Some(completed);
                    fps
                } else {
                    None
                };
                let result = ResultFrame {
                    generation: input.generation,
                    frame_epoch: input.frame_epoch,
                    source: input.source,
                    preview,
                    count,
                    model_ms: output.model_ms,
                    processing_ms: completed.duration_since(started).as_secs_f64() * 1000.0,
                    processed_fps,
                    age_ms: (input.source == Source::Live)
                        .then(|| completed.duration_since(input.captured_at).as_millis()),
                };
                let _ = sender.send(Command::InferenceEvent(Event::Result(result)));
            }
            Err(error) => {
                let event = if input.source == Source::Image {
                    Event::ImageFailed(
                        input.generation,
                        input.frame_epoch,
                        ImageFailure::Inference(error),
                    )
                } else {
                    session = None;
                    Event::Failed(input.generation, error)
                };
                let _ = sender.send(Command::InferenceEvent(event));
            }
        }
    }
}

struct Letterbox {
    data: Vec<f32>,
    width: usize,
    height: usize,
    scale: f32,
    pad_x: f32,
    pad_y: f32,
}

fn letterbox(frame: &Frame) -> Result<Letterbox, String> {
    if frame.width == 0
        || frame.height == 0
        || frame.rgb.len() != frame.width as usize * frame.height as usize * 3
    {
        return Err("Camera frame has invalid RGB dimensions".to_owned());
    }
    let scale =
        (INPUT_SIZE as f32 / frame.width as f32).min(INPUT_SIZE as f32 / frame.height as f32);
    let resized_width = (frame.width as f32 * scale).round() as usize;
    let resized_height = (frame.height as f32 * scale).round() as usize;
    let width = resized_width.div_ceil(32) * 32;
    let height = resized_height.div_ceil(32) * 32;
    let left = (width - resized_width) / 2;
    let top = (height - resized_height) / 2;
    let plane = width * height;
    let mut data = vec![114.0 / 255.0; plane * 3];
    for y in 0..resized_height {
        let sy = ((y as f32 + 0.5) * frame.height as f32 / resized_height as f32 - 0.5)
            .clamp(0.0, (frame.height - 1) as f32);
        let y0 = sy.floor() as usize;
        let y1 = (y0 + 1).min(frame.height as usize - 1);
        let fy = sy - y0 as f32;
        for x in 0..resized_width {
            let sx = ((x as f32 + 0.5) * frame.width as f32 / resized_width as f32 - 0.5)
                .clamp(0.0, (frame.width - 1) as f32);
            let x0 = sx.floor() as usize;
            let x1 = (x0 + 1).min(frame.width as usize - 1);
            let fx = sx - x0 as f32;
            let p00 = (y0 * frame.width as usize + x0) * 3;
            let p01 = (y0 * frame.width as usize + x1) * 3;
            let p10 = (y1 * frame.width as usize + x0) * 3;
            let p11 = (y1 * frame.width as usize + x1) * 3;
            let index = (y + top) * width + x + left;
            for channel in 0..3 {
                let a = frame.rgb[p00 + channel] as f32 * (1.0 - fx)
                    + frame.rgb[p01 + channel] as f32 * fx;
                let b = frame.rgb[p10 + channel] as f32 * (1.0 - fx)
                    + frame.rgb[p11 + channel] as f32 * fx;
                data[channel * plane + index] = (a * (1.0 - fy) + b * fy) / 255.0;
            }
        }
    }
    Ok(Letterbox {
        data,
        width,
        height,
        scale,
        pad_x: left as f32,
        pad_y: top as f32,
    })
}

fn infer(session: &mut Session, frame: &Frame) -> Result<InferenceOutput, String> {
    let preprocess_started = Instant::now();
    let input = letterbox(frame)?;
    let tensor = Tensor::from_array((vec![1, 3, input.height, input.width], input.data))
        .map_err(|error| format!("ONNX input: {error}"))?;
    let preprocess_ms = preprocess_started.elapsed().as_secs_f64() * 1000.0;
    let model_started = Instant::now();
    let outputs = session
        .run(ort::inputs![tensor])
        .map_err(|error| format!("ONNX inference: {error}"))?;
    let model_ms = model_started.elapsed().as_secs_f64() * 1000.0;
    let decode_started = Instant::now();
    let (shape, values) = outputs[0]
        .try_extract_tensor::<f32>()
        .map_err(|error| format!("YOLO prediction tensor: {error}"))?;
    if shape.len() != 3 || shape[0] != 1 {
        return Err(format!("Unexpected YOLO output shape: {shape:?}"));
    }
    let a = usize::try_from(shape[1]).map_err(|_| "Invalid YOLO output shape".to_owned())?;
    let b = usize::try_from(shape[2]).map_err(|_| "Invalid YOLO output shape".to_owned())?;
    let (channels_first, predictions) = if a == 37 {
        (true, b)
    } else if b == 37 {
        (false, a)
    } else {
        return Err(format!(
            "Expected a one-class YOLO11 segmentation head (37 channels), got {shape:?}"
        ));
    };
    let at = |prediction: usize, channel: usize| -> f32 {
        if channels_first {
            values[channel * predictions + prediction]
        } else {
            values[prediction * 37 + channel]
        }
    };
    let mut candidates = Vec::new();
    for index in 0..predictions {
        let score = at(index, 4);
        if !score.is_finite() || score < CONFIDENCE {
            continue;
        }
        let cx = at(index, 0);
        let cy = at(index, 1);
        let width = at(index, 2);
        let height = at(index, 3);
        if !cx.is_finite()
            || !cy.is_finite()
            || !width.is_finite()
            || !height.is_finite()
            || width <= 0.0
            || height <= 0.0
        {
            continue;
        }
        let x1 = ((cx - width / 2.0 - input.pad_x) / input.scale).clamp(0.0, frame.width as f32);
        let y1 = ((cy - height / 2.0 - input.pad_y) / input.scale).clamp(0.0, frame.height as f32);
        let x2 = ((cx + width / 2.0 - input.pad_x) / input.scale).clamp(0.0, frame.width as f32);
        let y2 = ((cy + height / 2.0 - input.pad_y) / input.scale).clamp(0.0, frame.height as f32);
        if x2 <= x1 || y2 <= y1 {
            continue;
        }
        candidates.push(Detection {
            center_x: (x1 + x2) / 2.0,
            center_y: (y1 + y2) / 2.0,
            x1,
            y1,
            x2,
            y2,
            score,
        });
    }
    candidates.sort_unstable_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(Ordering::Equal));
    let mut selected = Vec::new();
    for candidate in candidates {
        if selected
            .iter()
            .all(|previous| overlap(candidate, *previous) <= IOU_THRESHOLD)
        {
            selected.push(candidate);
            if selected.len() >= MAX_DETECTIONS {
                break;
            }
        }
    }
    Ok(InferenceOutput {
        detections: selected,
        preprocess_ms,
        model_ms,
        decode_ms: decode_started.elapsed().as_secs_f64() * 1000.0,
    })
}

fn overlap(a: Detection, b: Detection) -> f32 {
    let intersection =
        (a.x2.min(b.x2) - a.x1.max(b.x1)).max(0.0) * (a.y2.min(b.y2) - a.y1.max(b.y1)).max(0.0);
    let area_a = (a.x2 - a.x1) * (a.y2 - a.y1);
    let area_b = (b.x2 - b.x1) * (b.y2 - b.y1);
    intersection / (area_a + area_b - intersection).max(f32::EPSILON)
}

pub fn annotate(frame: &Frame, detections: &[Detection]) -> Frame {
    let mut rgb = frame.rgb.clone();
    let scale = frame.width.div_ceil(1024).max(frame.height.div_ceil(768));
    let radius = (scale * 5).clamp(5, 35) as i32;
    for detection in detections {
        let cx = detection.center_x.round() as i32;
        let cy = detection.center_y.round() as i32;
        for dy in -radius..=radius {
            for dx in -radius..=radius {
                if dx * dx + dy * dy > radius * radius {
                    continue;
                }
                let x = cx + dx;
                let y = cy + dy;
                if x < 0 || y < 0 || x >= frame.width as i32 || y >= frame.height as i32 {
                    continue;
                }
                let offset = (y as usize * frame.width as usize + x as usize) * 3;
                if dx * dx + dy * dy < (radius / 3) * (radius / 3) {
                    rgb[offset..offset + 3].copy_from_slice(&[255, 255, 255]);
                } else {
                    rgb[offset..offset + 3].copy_from_slice(&[235, 39, 52]);
                }
            }
        }
    }
    Frame {
        width: frame.width,
        height: frame.height,
        rgb,
    }
}

fn preview(frame: &Frame, transform: ImageTransform) -> Frame {
    let (transformed_width, transformed_height) = transform.dimensions(frame.width, frame.height);
    let factor = transformed_width
        .div_ceil(1024)
        .max(transformed_height.div_ceil(768));
    let width = transformed_width.div_ceil(factor);
    let height = transformed_height.div_ceil(factor);
    let mut rgb = vec![0u8; width as usize * height as usize * 3];
    for y in 0..height {
        for x in 0..width {
            let (sx, sy) = transform.source(x * factor, y * factor, frame.width, frame.height);
            let source = (sy as usize * frame.width as usize + sx as usize) * 3;
            let destination = (y as usize * width as usize + x as usize) * 3;
            rgb[destination..destination + 3].copy_from_slice(&frame.rgb[source..source + 3]);
        }
    }
    Frame { width, height, rgb }
}

pub fn infer_image(
    model_path: &std::path::Path,
    image_path: &std::path::Path,
    output_path: Option<&std::path::Path>,
) -> Result<(), String> {
    let image = image::open(image_path)
        .map_err(|error| format!("Could not read sample image: {error}"))?
        .to_rgb8();
    let frame = Frame {
        width: image.width(),
        height: image.height(),
        rgb: image.into_raw(),
    };
    let environment = ort::init()
        .build()
        .map_err(|error| format!("ONNX Runtime: {error}"))?;
    let mut session = Session::builder(&environment)
        .and_then(|mut builder| builder.commit_from_file(model_path))
        .map_err(|error| format!("Could not open ONNX model: {error}"))?;
    let start = Instant::now();
    let output = infer(&mut session, &frame)?;
    let annotate_started = Instant::now();
    let marked = annotate(&frame, &output.detections);
    let annotate_ms = annotate_started.elapsed().as_secs_f64() * 1000.0;
    let preview_started = Instant::now();
    let displayed = preview(&marked, ImageTransform::default());
    let preview_ms = preview_started.elapsed().as_secs_f64() * 1000.0;
    let milliseconds = start.elapsed().as_secs_f64() * 1000.0;
    println!(
        "ONNX count: {} | model: {:.0} ms | processing: {:.0} ms | image: {}x{} | preview: {}x{}",
        output.detections.len(),
        output.model_ms,
        milliseconds,
        frame.width,
        frame.height,
        displayed.width,
        displayed.height,
    );
    println!(
        "Stages: prepare {:.1} ms | model {:.1} ms | decode/NMS {:.1} ms | annotate {:.1} ms | preview {:.1} ms",
        output.preprocess_ms, output.model_ms, output.decode_ms, annotate_ms, preview_ms,
    );
    if let Some(path) = output_path {
        image::save_buffer(
            path,
            &marked.rgb,
            marked.width,
            marked.height,
            image::ColorType::Rgb8,
        )
        .map_err(|error| format!("Could not save marked sample: {error}"))?;
        println!("Marked sample: {}", path.display());
    }
    Ok(())
}

pub fn benchmark_image(
    model_path: &std::path::Path,
    image_path: &std::path::Path,
    runs: usize,
) -> Result<(), String> {
    if !(1..=1000).contains(&runs) {
        return Err("Run count must be between 1 and 1000".to_owned());
    }
    let image = image::open(image_path)
        .map_err(|error| format!("Could not read benchmark image: {error}"))?
        .to_rgb8();
    let frame = Frame {
        width: image.width(),
        height: image.height(),
        rgb: image.into_raw(),
    };
    let environment = ort::init()
        .build()
        .map_err(|error| format!("ONNX Runtime: {error}"))?;
    let mut session = Session::builder(&environment)
        .and_then(|mut builder| builder.commit_from_file(model_path))
        .map_err(|error| format!("Could not open ONNX model: {error}"))?;

    // Warm up the loaded session before timing steady-state frame processing.
    for _ in 0..2 {
        let output = infer(&mut session, &frame)?;
        let marked = annotate(&frame, &output.detections);
        std::hint::black_box(preview(&marked, ImageTransform::default()));
    }
    let started = Instant::now();
    let mut total_model_ms = 0.0;
    let mut total_processing_ms = 0.0;
    let mut count = None;
    for _ in 0..runs {
        let frame_started = Instant::now();
        let output = infer(&mut session, &frame)?;
        let marked = annotate(&frame, &output.detections);
        std::hint::black_box(preview(&marked, ImageTransform::default()));
        if let Some(previous) = count {
            if previous != output.detections.len() {
                return Err("Count changed between benchmark frames".to_owned());
            }
        }
        count = Some(output.detections.len());
        total_model_ms += output.model_ms;
        total_processing_ms += frame_started.elapsed().as_secs_f64() * 1000.0;
    }
    let elapsed = started.elapsed().as_secs_f64();
    println!(
        "Benchmark: {runs} frames | count: {} | model: {:.1} ms/frame | processing: {:.1} ms/frame | throughput: {:.1} FPS",
        count.unwrap_or_default(),
        total_model_ms / runs as f64,
        total_processing_ms / runs as f64,
        runs as f64 / elapsed,
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{Event, ImageFailure, Runner, Source};
    use crate::worker::Command;
    use std::path::PathBuf;
    use std::sync::mpsc;
    use std::time::Duration;

    #[test]
    #[ignore = "requires BASLER_TEST_ONNX, BASLER_TEST_IMAGE and BASLER_TEST_EMPTY_IMAGE outside the repository"]
    fn selected_image_runs_without_a_camera() {
        let model = PathBuf::from(std::env::var_os("BASLER_TEST_ONNX").expect("model path"));
        let image = PathBuf::from(std::env::var_os("BASLER_TEST_IMAGE").expect("image path"));
        let empty_image =
            PathBuf::from(std::env::var_os("BASLER_TEST_EMPTY_IMAGE").expect("empty image path"));
        let (sender, receiver) = mpsc::channel();
        let runner = Runner::new(sender).expect("start ONNX worker");
        let generation = runner.load(model);
        match receiver
            .recv_timeout(Duration::from_secs(30))
            .expect("load event")
        {
            Command::InferenceEvent(Event::Loaded(received, _)) => {
                assert_eq!(received, generation);
            }
            _ => panic!("ONNX model did not load"),
        }
        let empty_epoch = runner.test_image(empty_image);
        match receiver
            .recv_timeout(Duration::from_secs(30))
            .expect("empty image result")
        {
            Command::InferenceEvent(Event::ImageFailed(received, epoch, ImageFailure::Empty)) => {
                assert_eq!(received, generation);
                assert_eq!(epoch, empty_epoch);
            }
            _ => panic!("empty image did not produce a specific error"),
        }
        let missing_image = std::env::temp_dir().join(format!(
            "basler_missing_test_image_{}.bmp",
            std::process::id()
        ));
        let invalid_epoch = runner.test_image(missing_image);
        match receiver
            .recv_timeout(Duration::from_secs(30))
            .expect("invalid image result")
        {
            Command::InferenceEvent(Event::ImageFailed(received, epoch, _)) => {
                assert_eq!(received, generation);
                assert_eq!(epoch, invalid_epoch);
            }
            _ => panic!("invalid image did not produce an error"),
        }
        let epoch = runner.test_image(image);
        match receiver
            .recv_timeout(Duration::from_secs(30))
            .expect("image result")
        {
            Command::InferenceEvent(Event::Result(result)) => {
                assert_eq!(result.generation, generation);
                assert_eq!(result.frame_epoch, epoch);
                assert!(result.source == Source::Image);
                assert_eq!(result.count, 68);
                assert!(result.model_ms > 0.0);
                assert!(result.model_ms <= result.processing_ms);
                assert!(result.processed_fps.is_none());
                assert!(result.preview.width > 0 && result.preview.height > 0);
            }
            _ => panic!("test image inference did not finish"),
        }
    }
}
