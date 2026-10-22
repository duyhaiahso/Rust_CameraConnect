use crate::camera::Frame;
use crate::transform::ImageTransform;
use crate::worker::Command;
use ort::session::Session;
use ort::value::Tensor;
use std::cmp::Ordering;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

const FALLBACK_INPUT_SIZE: InputSize = InputSize {
    width: 1600,
    height: 1600,
};
const MIN_INPUT_SIDE: u32 = 64;
const MAX_INPUT_SIDE: u32 = 2048;
const CONFIDENCE: f32 = 0.25;
const IOU_THRESHOLD: f32 = 0.7;
const MAX_DETECTIONS: usize = 300;
const DOT_COLOR: [u8; 3] = [235, 39, 52];
const BOX_COLOR: [u8; 3] = [36, 220, 80];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AnnotationMode {
    Dot,
    Box,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InputSize {
    pub width: u32,
    pub height: u32,
}

impl InputSize {
    fn valid(self) -> bool {
        [self.width, self.height].into_iter().all(|side| {
            (MIN_INPUT_SIDE..=MAX_INPUT_SIDE).contains(&side) && side.is_multiple_of(32)
        })
    }

    fn from_metadata(value: &str) -> Option<Self> {
        let value = value.trim().trim_matches(['[', ']', '(', ')']);
        let parts = value.split(',').map(str::trim).collect::<Vec<_>>();
        let size = match parts.as_slice() {
            [side] => {
                let side = side.parse().ok()?;
                Self {
                    width: side,
                    height: side,
                }
            }
            [height, width] => Self {
                width: width.parse().ok()?,
                height: height.parse().ok()?,
            },
            _ => return None,
        };
        size.valid().then_some(size)
    }

    pub fn from_user(value: &str) -> Option<Self> {
        let value = value.trim().replace('×', "x");
        let parts = value.split('x').map(str::trim).collect::<Vec<_>>();
        let size = match parts.as_slice() {
            [side] => {
                let side = side.parse().ok()?;
                Self {
                    width: side,
                    height: side,
                }
            }
            [width, height] => Self {
                width: width.parse().ok()?,
                height: height.parse().ok()?,
            },
            _ => return None,
        };
        size.valid().then_some(size)
    }

    pub fn label(self) -> String {
        format!("{}×{}", self.width, self.height)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SizeSource {
    Fallback,
    Metadata,
    Fixed,
}

#[derive(Clone, Copy)]
pub struct ModelInput {
    pub active: InputSize,
    pub default: InputSize,
    pub source: SizeSource,
}

impl ModelInput {
    fn fallback() -> Self {
        Self {
            active: FALLBACK_INPUT_SIZE,
            default: FALLBACK_INPUT_SIZE,
            source: SizeSource::Fallback,
        }
    }

    fn from_session(session: &Session) -> Result<Self, String> {
        let shape = session
            .inputs()
            .first()
            .ok_or("ONNX model has no image input")?
            .dtype()
            .tensor_shape();
        if let Some(shape) = shape.filter(|shape| shape.len() == 4) {
            let (height, width) = (shape[2], shape[3]);
            if height > 0 || width > 0 {
                if height <= 0 || width <= 0 {
                    return Err("Partly fixed ONNX image dimensions are unsupported".to_owned());
                }
                let fixed = InputSize {
                    width: u32::try_from(width).map_err(|_| "ONNX input width is too large")?,
                    height: u32::try_from(height).map_err(|_| "ONNX input height is too large")?,
                };
                if !fixed.valid() {
                    return Err(
                        "Fixed ONNX input dimensions must be multiples of 32 from 64 to 2048"
                            .to_owned(),
                    );
                }
                return Ok(Self {
                    active: fixed,
                    default: fixed,
                    source: SizeSource::Fixed,
                });
            }
        }
        let metadata = session
            .metadata()
            .ok()
            .and_then(|model| model.custom("imgsz"));
        if let Some(size) = metadata.as_deref().and_then(InputSize::from_metadata) {
            Ok(Self {
                active: size,
                default: size,
                source: SizeSource::Metadata,
            })
        } else {
            Ok(Self::fallback())
        }
    }
}

#[derive(Clone, Copy)]
pub struct Detection {
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
    pub detail_frame: Arc<Frame>,
    pub transform: ImageTransform,
    pub count: usize,
    pub model_ms: f64,
    pub processing_ms: f64,
    pub processed_fps: Option<f64>,
    pub age_ms: Option<u128>,
    pub image_path: Option<PathBuf>,
    pub continuous: bool,
    pub processed_images: u64,
}

#[derive(Clone)]
pub enum LoopInput {
    Image(PathBuf),
    Folder(PathBuf),
}

impl LoopInput {
    pub fn path(&self) -> &std::path::Path {
        match self {
            Self::Image(path) | Self::Folder(path) => path,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Live,
    Image,
}

pub enum Event {
    Loaded(u64, PathBuf, ModelInput),
    SizeChanged(u64, u64, InputSize),
    AnnotationModeChanged(u64, u64, AnnotationMode),
    Unloaded(u64),
    Failed(u64, String),
    ImageFailed(u64, u64, ImageFailure),
    LoopSkipped(u64, u64, PathBuf, String),
    LoopFailed(u64, u64, String),
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
    image_path: Option<PathBuf>,
    continuous: bool,
}

enum Action {
    Load(u64, PathBuf),
    Unload(u64),
    TestImage(u64, u64, PathBuf),
    StartLoop(u64, u64, LoopInput),
    StopLoop,
    SetInputSize(u64, u64, InputSize),
    SetAnnotationMode(u64, u64, AnnotationMode),
}

enum LoopFiles {
    Image(PathBuf, Arc<Frame>),
    Folder(Vec<PathBuf>),
}

struct LoopState {
    generation: u64,
    frame_epoch: u64,
    files: LoopFiles,
    index: usize,
    failed_in_row: usize,
    reported_errors: HashSet<PathBuf>,
    started_at: Instant,
    completed: u64,
}

enum LoopStep {
    Frame(InputFrame),
    Skip(PathBuf, String, bool),
    Exhausted(PathBuf, String, bool),
}

fn read_image(path: &std::path::Path) -> Result<Arc<Frame>, ImageFailure> {
    let metadata =
        std::fs::metadata(path).map_err(|error| ImageFailure::Read(error.to_string()))?;
    if metadata.len() == 0 {
        return Err(ImageFailure::Empty);
    }
    let image = image::open(path).map_err(|error| ImageFailure::Read(error.to_string()))?;
    let rgb = image.to_rgb8();
    Ok(Arc::new(Frame {
        width: rgb.width(),
        height: rgb.height(),
        rgb: rgb.into_raw(),
    }))
}

impl LoopState {
    fn open(generation: u64, frame_epoch: u64, input: LoopInput) -> Result<Self, String> {
        let files = match input {
            LoopInput::Image(path) => {
                let frame = read_image(&path).map_err(|error| error.detail())?;
                LoopFiles::Image(path, frame)
            }
            LoopInput::Folder(folder) => {
                let entries = std::fs::read_dir(&folder)
                    .map_err(|error| format!("Could not read image folder: {error}"))?;
                let mut files = Vec::new();
                for entry in entries {
                    let entry =
                        entry.map_err(|error| format!("Could not list image folder: {error}"))?;
                    if !entry.file_type().is_ok_and(|kind| kind.is_file()) {
                        continue;
                    }
                    let path = entry.path();
                    if path
                        .extension()
                        .and_then(|value| value.to_str())
                        .is_some_and(|extension| {
                            ["bmp", "png", "jpg", "jpeg"]
                                .iter()
                                .any(|allowed| extension.eq_ignore_ascii_case(allowed))
                        })
                    {
                        files.push(path);
                    }
                }
                files.sort_by(|a, b| a.file_name().cmp(&b.file_name()));
                if files.is_empty() {
                    return Err("The folder contains no BMP, PNG or JPEG images".to_owned());
                }
                LoopFiles::Folder(files)
            }
        };
        Ok(Self {
            generation,
            frame_epoch,
            files,
            index: 0,
            failed_in_row: 0,
            reported_errors: HashSet::new(),
            started_at: Instant::now(),
            completed: 0,
        })
    }

    fn next(&mut self) -> LoopStep {
        let (path, frame) = match &self.files {
            LoopFiles::Image(path, frame) => (path.clone(), Ok(Arc::clone(frame))),
            LoopFiles::Folder(files) => {
                let path = files[self.index].clone();
                self.index = (self.index + 1) % files.len();
                let frame = read_image(&path);
                (path, frame)
            }
        };
        match frame {
            Ok(frame) => {
                self.failed_in_row = 0;
                LoopStep::Frame(InputFrame {
                    generation: self.generation,
                    frame_epoch: self.frame_epoch,
                    source: Source::Image,
                    frame,
                    transform: ImageTransform::default(),
                    captured_at: Instant::now(),
                    image_path: Some(path),
                    continuous: true,
                })
            }
            Err(error) => {
                self.failed_in_row += 1;
                let detail = error.detail();
                let first_error = self.reported_errors.insert(path.clone());
                if matches!(&self.files, LoopFiles::Folder(files) if self.failed_in_row >= files.len())
                {
                    LoopStep::Exhausted(path, detail, first_error)
                } else {
                    LoopStep::Skip(path, detail, first_error)
                }
            }
        }
    }
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
                image_path: None,
                continuous: false,
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

    pub fn start_loop(&self, input: LoopInput) -> u64 {
        let (lock, wake) = &*self.shared;
        if let Ok(mut state) = lock.lock() {
            state.frame_epoch += 1;
            state.latest = None;
            state.action = Some(Action::StartLoop(
                state.generation,
                state.frame_epoch,
                input,
            ));
            wake.notify_one();
            state.frame_epoch
        } else {
            0
        }
    }

    pub fn stop_loop(&self) -> u64 {
        let (lock, wake) = &*self.shared;
        if let Ok(mut state) = lock.lock() {
            state.frame_epoch += 1;
            state.latest = None;
            state.action = Some(Action::StopLoop);
            wake.notify_one();
            state.frame_epoch
        } else {
            0
        }
    }

    pub fn set_input_size(&self, size: InputSize) -> u64 {
        let (lock, wake) = &*self.shared;
        if let Ok(mut state) = lock.lock() {
            state.frame_epoch += 1;
            state.latest = None;
            state.action = Some(Action::SetInputSize(
                state.generation,
                state.frame_epoch,
                size,
            ));
            wake.notify_one();
            state.frame_epoch
        } else {
            0
        }
    }

    pub fn set_annotation_mode(&self, mode: AnnotationMode) -> u64 {
        let (lock, wake) = &*self.shared;
        if let Ok(mut state) = lock.lock() {
            state.frame_epoch += 1;
            state.latest = None;
            state.action = Some(Action::SetAnnotationMode(
                state.generation,
                state.frame_epoch,
                mode,
            ));
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
    let mut model_input = ModelInput::fallback();
    let mut annotation_mode = AnnotationMode::Box;
    let mut environment = None;
    let mut previous_completed: Option<Instant> = None;
    let mut image_loop: Option<LoopState> = None;
    loop {
        let (action, input) = {
            let (lock, wake) = &*shared;
            let mut state = match lock.lock() {
                Ok(state) => state,
                Err(_) => return,
            };
            while !state.shutdown
                && state.action.is_none()
                && state.latest.is_none()
                && image_loop.is_none()
            {
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
                    image_loop = None;
                    session = None;
                    previous_completed = None;
                    let opened = (|| -> Result<(Session, ModelInput), String> {
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
                        let input = ModelInput::from_session(&loaded)?;
                        Ok((loaded, input))
                    })();
                    match opened {
                        Ok((loaded, input)) => {
                            model_input = input;
                            session = Some(loaded);
                            let _ = sender.send(Command::InferenceEvent(Event::Loaded(
                                generation, path, input,
                            )));
                        }
                        Err(error) => {
                            let _ = sender
                                .send(Command::InferenceEvent(Event::Failed(generation, error)));
                        }
                    }
                    continue;
                }
                Action::Unload(generation) => {
                    image_loop = None;
                    session = None;
                    model_input = ModelInput::fallback();
                    previous_completed = None;
                    let _ = sender.send(Command::InferenceEvent(Event::Unloaded(generation)));
                    continue;
                }
                Action::TestImage(generation, frame_epoch, path) => {
                    image_loop = None;
                    previous_completed = None;
                    match read_image(&path) {
                        Ok(frame) => Some(InputFrame {
                            generation,
                            frame_epoch,
                            source: Source::Image,
                            frame,
                            transform: ImageTransform::default(),
                            captured_at: Instant::now(),
                            image_path: Some(path),
                            continuous: false,
                        }),
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
                Action::StartLoop(generation, frame_epoch, selection) => {
                    previous_completed = None;
                    match LoopState::open(generation, frame_epoch, selection) {
                        Ok(state) => image_loop = Some(state),
                        Err(error) => {
                            image_loop = None;
                            let _ = sender.send(Command::InferenceEvent(Event::LoopFailed(
                                generation,
                                frame_epoch,
                                error,
                            )));
                        }
                    }
                    None
                }
                Action::StopLoop => {
                    image_loop = None;
                    previous_completed = None;
                    None
                }
                Action::SetInputSize(generation, frame_epoch, size) => {
                    if session.is_some() && model_input.source != SizeSource::Fixed {
                        model_input.active = size;
                        previous_completed = None;
                        if let Some(looping) = image_loop.as_mut() {
                            looping.frame_epoch = frame_epoch;
                            looping.started_at = Instant::now();
                            looping.completed = 0;
                        }
                        let _ = sender.send(Command::InferenceEvent(Event::SizeChanged(
                            generation,
                            frame_epoch,
                            size,
                        )));
                    }
                    None
                }
                Action::SetAnnotationMode(generation, frame_epoch, mode) => {
                    annotation_mode = mode;
                    previous_completed = None;
                    if let Some(looping) = image_loop.as_mut() {
                        looping.frame_epoch = frame_epoch;
                        looping.started_at = Instant::now();
                        looping.completed = 0;
                    }
                    let _ = sender.send(Command::InferenceEvent(Event::AnnotationModeChanged(
                        generation,
                        frame_epoch,
                        mode,
                    )));
                    None
                }
            }
        } else {
            input
        };
        let input = if input.is_some() {
            input
        } else if let Some(looping) = image_loop.as_mut() {
            match looping.next() {
                LoopStep::Frame(frame) => Some(frame),
                LoopStep::Skip(path, error, first_error) => {
                    if first_error {
                        let _ = sender.send(Command::InferenceEvent(Event::LoopSkipped(
                            looping.generation,
                            looping.frame_epoch,
                            path,
                            error,
                        )));
                    }
                    None
                }
                LoopStep::Exhausted(path, error, first_error) => {
                    if first_error {
                        let _ = sender.send(Command::InferenceEvent(Event::LoopSkipped(
                            looping.generation,
                            looping.frame_epoch,
                            path,
                            error,
                        )));
                    }
                    let _ = sender.send(Command::InferenceEvent(Event::LoopFailed(
                        looping.generation,
                        looping.frame_epoch,
                        "The folder has no readable images".to_owned(),
                    )));
                    image_loop = None;
                    None
                }
            }
        } else {
            None
        };
        let (Some(input), Some(model)) = (input, session.as_mut()) else {
            continue;
        };
        let started = Instant::now();
        match infer(
            model,
            &input.frame,
            model_input.active,
            model_input.source == SizeSource::Fixed,
        ) {
            Ok(output) => {
                let count = output.detections.len();
                let annotated =
                    Arc::new(annotate(&input.frame, &output.detections, annotation_mode));
                let preview = preview(&annotated, input.transform);
                let completed = Instant::now();
                let processed_fps = if input.continuous {
                    image_loop.as_mut().map(|looping| {
                        looping.completed += 1;
                        looping.completed as f64
                            / completed.duration_since(looping.started_at).as_secs_f64()
                    })
                } else if input.source == Source::Live {
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
                    detail_frame: annotated,
                    transform: input.transform,
                    count,
                    model_ms: output.model_ms,
                    processing_ms: completed.duration_since(started).as_secs_f64() * 1000.0,
                    processed_fps,
                    age_ms: (input.source == Source::Live)
                        .then(|| completed.duration_since(input.captured_at).as_millis()),
                    image_path: input.image_path,
                    continuous: input.continuous,
                    processed_images: image_loop.as_ref().map_or(0, |looping| looping.completed),
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
                    image_loop = None;
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

fn letterbox(frame: &Frame, target: InputSize, pad_to_full: bool) -> Result<Letterbox, String> {
    if frame.width == 0
        || frame.height == 0
        || frame.rgb.len() != frame.width as usize * frame.height as usize * 3
    {
        return Err("Camera frame has invalid RGB dimensions".to_owned());
    }
    let scale =
        (target.width as f32 / frame.width as f32).min(target.height as f32 / frame.height as f32);
    let resized_width = (frame.width as f32 * scale).round() as usize;
    let resized_height = (frame.height as f32 * scale).round() as usize;
    let width = if pad_to_full {
        target.width as usize
    } else {
        resized_width.div_ceil(32) * 32
    };
    let height = if pad_to_full {
        target.height as usize
    } else {
        resized_height.div_ceil(32) * 32
    };
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

fn infer(
    session: &mut Session,
    frame: &Frame,
    size: InputSize,
    pad_to_full: bool,
) -> Result<InferenceOutput, String> {
    let preprocess_started = Instant::now();
    let input = letterbox(frame, size, pad_to_full)?;
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

pub fn annotate(frame: &Frame, detections: &[Detection], mode: AnnotationMode) -> Frame {
    let mut rgb = frame.rgb.clone();
    let scale = frame.width.div_ceil(1024).max(frame.height.div_ceil(768));
    let preview_border = scale.max(2) as usize;
    for detection in detections {
        if mode == AnnotationMode::Dot {
            let radius = (scale * 5).clamp(5, 35) as i32;
            let cx = ((detection.x1 + detection.x2) / 2.0).round() as i32;
            let cy = ((detection.y1 + detection.y2) / 2.0).round() as i32;
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
                        rgb[offset..offset + 3].copy_from_slice(&DOT_COLOR);
                    }
                }
            }
            continue;
        }
        let left = detection.x1.floor().clamp(0.0, frame.width as f32) as usize;
        let top = detection.y1.floor().clamp(0.0, frame.height as f32) as usize;
        let right = detection.x2.ceil().clamp(0.0, frame.width as f32) as usize;
        let bottom = detection.y2.ceil().clamp(0.0, frame.height as f32) as usize;
        if left >= right || top >= bottom {
            continue;
        }
        let border = preview_border
            .min((right - left).div_ceil(3))
            .min((bottom - top).div_ceil(3));
        for y in top..bottom {
            let row_start = y * frame.width as usize * 3;
            if y - top < border || bottom - 1 - y < border {
                for pixel in rgb[row_start + left * 3..row_start + right * 3].chunks_exact_mut(3) {
                    pixel.copy_from_slice(&BOX_COLOR);
                }
            } else {
                for x in left..left + border {
                    let offset = row_start + x * 3;
                    rgb[offset..offset + 3].copy_from_slice(&BOX_COLOR);
                }
                for x in right - border..right {
                    let offset = row_start + x * 3;
                    rgb[offset..offset + 3].copy_from_slice(&BOX_COLOR);
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
    let model_input = ModelInput::from_session(&session)?;
    let start = Instant::now();
    let output = infer(
        &mut session,
        &frame,
        model_input.active,
        model_input.source == SizeSource::Fixed,
    )?;
    let annotate_started = Instant::now();
    let marked = annotate(&frame, &output.detections, AnnotationMode::Box);
    let annotate_ms = annotate_started.elapsed().as_secs_f64() * 1000.0;
    let preview_started = Instant::now();
    let displayed = preview(&marked, ImageTransform::default());
    let preview_ms = preview_started.elapsed().as_secs_f64() * 1000.0;
    let milliseconds = start.elapsed().as_secs_f64() * 1000.0;
    println!(
        "ONNX count: {} | model: {:.0} ms | processing: {:.0} ms | image: {}x{} | input: {} | preview: {}x{}",
        output.detections.len(),
        output.model_ms,
        milliseconds,
        frame.width,
        frame.height,
        model_input.active.label(),
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
    override_size: Option<InputSize>,
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
    let model_input = ModelInput::from_session(&session)?;
    if override_size.is_some() && model_input.source == SizeSource::Fixed {
        return Err("Cannot override a fixed ONNX input size".to_owned());
    }
    let size = override_size.unwrap_or(model_input.active);

    // Warm up the loaded session before timing steady-state frame processing.
    for _ in 0..2 {
        let output = infer(
            &mut session,
            &frame,
            size,
            model_input.source == SizeSource::Fixed,
        )?;
        let marked = annotate(&frame, &output.detections, AnnotationMode::Box);
        std::hint::black_box(preview(&marked, ImageTransform::default()));
    }
    let started = Instant::now();
    let mut total_model_ms = 0.0;
    let mut total_processing_ms = 0.0;
    let mut count = None;
    for _ in 0..runs {
        let frame_started = Instant::now();
        let output = infer(
            &mut session,
            &frame,
            size,
            model_input.source == SizeSource::Fixed,
        )?;
        let marked = annotate(&frame, &output.detections, AnnotationMode::Box);
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
        "Benchmark: {runs} frames | input: {} | count: {} | model: {:.1} ms/frame | processing: {:.1} ms/frame | throughput: {:.1} FPS",
        size.label(),
        count.unwrap_or_default(),
        total_model_ms / runs as f64,
        total_processing_ms / runs as f64,
        runs as f64 / elapsed,
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        AnnotationMode, BOX_COLOR, DOT_COLOR, Detection, Event, ImageFailure, InputSize, LoopInput,
        Runner, Source, annotate,
    };
    use crate::camera::Frame;
    use crate::worker::Command;
    use std::path::PathBuf;
    use std::sync::mpsc;
    use std::time::Duration;

    #[test]
    fn annotation_modes_keep_the_same_detection_and_use_distinct_colors() {
        let frame = Frame {
            width: 24,
            height: 24,
            rgb: vec![0; 24 * 24 * 3],
        };
        let detection = Detection {
            x1: 4.0,
            y1: 4.0,
            x2: 20.0,
            y2: 20.0,
            score: 0.9,
        };
        let dot = annotate(&frame, &[detection], AnnotationMode::Dot);
        let bbox = annotate(&frame, &[detection], AnnotationMode::Box);
        fn pixel(frame: &Frame, x: usize, y: usize) -> &[u8] {
            let start = (y * frame.width as usize + x) * 3;
            &frame.rgb[start..start + 3]
        }
        assert_eq!(pixel(&dot, 12, 12), &[255, 255, 255]);
        assert_eq!(pixel(&dot, 16, 12), &DOT_COLOR);
        assert_eq!(pixel(&dot, 4, 4), &[0, 0, 0]);
        assert_eq!(pixel(&bbox, 4, 4), &BOX_COLOR);
        assert_eq!(pixel(&bbox, 12, 12), &[0, 0, 0]);
        assert!(frame.rgb.iter().all(|value| *value == 0));
    }

    #[test]
    fn parses_and_bounds_model_input_sizes() {
        assert_eq!(
            InputSize::from_metadata("[704, 704]"),
            Some(InputSize {
                width: 704,
                height: 704
            })
        );
        assert_eq!(
            InputSize::from_metadata("[640, 960]"),
            Some(InputSize {
                width: 960,
                height: 640
            })
        );
        assert_eq!(
            InputSize::from_user("704"),
            Some(InputSize {
                width: 704,
                height: 704
            })
        );
        assert_eq!(
            InputSize::from_user("960×640"),
            Some(InputSize {
                width: 960,
                height: 640
            })
        );
        assert_eq!(InputSize::from_user("1601"), None);
        assert_eq!(InputSize::from_metadata("[4096, 4096]"), None);
    }

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
            Command::InferenceEvent(Event::Loaded(received, _, _)) => {
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

    #[test]
    #[ignore = "requires BASLER_TEST_ONNX and BASLER_TEST_IMAGE outside the repository"]
    fn continuous_image_and_folder_can_switch_back_to_one_shot() {
        let model = PathBuf::from(std::env::var_os("BASLER_TEST_ONNX").expect("model path"));
        let image = PathBuf::from(std::env::var_os("BASLER_TEST_IMAGE").expect("image path"));
        let folder = std::env::temp_dir().join(format!(
            "basler-size-loop-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        std::fs::create_dir(&folder).expect("create loop test folder");
        std::fs::copy(&image, folder.join("sample.bmp")).expect("copy valid sample");
        std::fs::write(folder.join("empty.bmp"), []).expect("create empty sample");
        let (sender, receiver) = mpsc::channel();
        let runner = Runner::new(sender).expect("start ONNX worker");
        let generation = runner.load(model);
        assert!(matches!(
            receiver.recv_timeout(Duration::from_secs(30)),
            Ok(Command::InferenceEvent(Event::Loaded(received, _, _))) if received == generation
        ));

        let image_epoch = runner.start_loop(LoopInput::Image(image.clone()));
        for expected in 1..=3 {
            match receiver
                .recv_timeout(Duration::from_secs(30))
                .expect("loop image result")
            {
                Command::InferenceEvent(Event::Result(result)) => {
                    assert_eq!(result.frame_epoch, image_epoch);
                    assert_eq!(result.count, 68);
                    assert!(result.continuous);
                    assert_eq!(result.processed_images, expected);
                    assert!(result.processed_fps.is_some_and(|fps| fps > 0.0));
                    assert_eq!(result.image_path.as_deref(), Some(image.as_path()));
                }
                _ => panic!("single-image loop did not return a result"),
            }
        }

        let changed_epoch = runner.set_input_size(InputSize {
            width: 704,
            height: 704,
        });
        let mut changed = false;
        loop {
            match receiver
                .recv_timeout(Duration::from_secs(30))
                .expect("resized loop result")
            {
                Command::InferenceEvent(Event::SizeChanged(received, epoch, size))
                    if received == generation && epoch == changed_epoch =>
                {
                    assert_eq!(
                        size,
                        InputSize {
                            width: 704,
                            height: 704
                        }
                    );
                    changed = true;
                }
                Command::InferenceEvent(Event::Result(result))
                    if result.frame_epoch == changed_epoch =>
                {
                    assert!(changed);
                    assert_eq!(result.processed_images, 1);
                    break;
                }
                _ => {}
            }
        }
        let restored_epoch = runner.set_input_size(InputSize {
            width: 1600,
            height: 1600,
        });
        loop {
            match receiver
                .recv_timeout(Duration::from_secs(30))
                .expect("restored loop result")
            {
                Command::InferenceEvent(Event::Result(result))
                    if result.frame_epoch == restored_epoch =>
                {
                    assert_eq!(result.count, 68);
                    assert_eq!(result.processed_images, 1);
                    break;
                }
                _ => {}
            }
        }

        let dot_epoch = runner.set_annotation_mode(AnnotationMode::Dot);
        let mut mode_changed = false;
        loop {
            match receiver
                .recv_timeout(Duration::from_secs(30))
                .expect("dot result")
            {
                Command::InferenceEvent(Event::AnnotationModeChanged(received, epoch, mode))
                    if received == generation && epoch == dot_epoch =>
                {
                    assert_eq!(mode, AnnotationMode::Dot);
                    mode_changed = true;
                }
                Command::InferenceEvent(Event::Result(result))
                    if result.frame_epoch == dot_epoch =>
                {
                    assert!(mode_changed);
                    assert_eq!(result.count, 68);
                    assert_eq!(result.processed_images, 1);
                    assert!(
                        result
                            .detail_frame
                            .rgb
                            .chunks_exact(3)
                            .any(|pixel| pixel == DOT_COLOR)
                    );
                    break;
                }
                _ => {}
            }
        }
        let box_epoch = runner.set_annotation_mode(AnnotationMode::Box);
        loop {
            match receiver
                .recv_timeout(Duration::from_secs(30))
                .expect("box result")
            {
                Command::InferenceEvent(Event::Result(result))
                    if result.frame_epoch == box_epoch =>
                {
                    assert_eq!(result.count, 68);
                    assert_eq!(result.processed_images, 1);
                    assert!(
                        result
                            .detail_frame
                            .rgb
                            .chunks_exact(3)
                            .any(|pixel| pixel == BOX_COLOR)
                    );
                    break;
                }
                _ => {}
            }
        }

        let folder_epoch = runner.start_loop(LoopInput::Folder(folder.clone()));
        let mut skipped = 0;
        let mut completed = 0;
        while completed < 2 {
            match receiver
                .recv_timeout(Duration::from_secs(30))
                .expect("folder result")
            {
                Command::InferenceEvent(Event::LoopSkipped(_, epoch, _, _))
                    if epoch == folder_epoch =>
                {
                    skipped += 1;
                }
                Command::InferenceEvent(Event::Result(result))
                    if result.frame_epoch == folder_epoch =>
                {
                    completed += 1;
                    assert!(result.continuous);
                    assert_eq!(result.processed_images, completed);
                    assert!(result.image_path.is_some());
                }
                Command::InferenceEvent(Event::LoopFailed(_, epoch, error))
                    if epoch == folder_epoch =>
                {
                    panic!("folder stopped unexpectedly: {error}");
                }
                _ => {} // A result from the previous loop may already be in the channel.
            }
        }
        assert!(skipped > 0, "the test folder should skip its 0-byte BMP");

        let stopped_epoch = runner.stop_loop();
        let single_epoch = runner.test_image(image.clone());
        assert!(single_epoch > stopped_epoch);
        loop {
            match receiver
                .recv_timeout(Duration::from_secs(30))
                .expect("one-shot result")
            {
                Command::InferenceEvent(Event::Result(result))
                    if result.frame_epoch == single_epoch =>
                {
                    assert_eq!(result.count, 68);
                    assert!(!result.continuous);
                    assert_eq!(result.processed_images, 0);
                    assert!(result.processed_fps.is_none());
                    break;
                }
                _ => {}
            }
        }

        let invalid_folder = std::env::temp_dir().join(format!(
            "basler-invalid-loop-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        std::fs::create_dir(&invalid_folder).expect("create test folder");
        std::fs::write(invalid_folder.join("empty.bmp"), []).expect("create empty BMP");
        let invalid_epoch = runner.start_loop(LoopInput::Folder(invalid_folder.clone()));
        loop {
            match receiver
                .recv_timeout(Duration::from_secs(30))
                .expect("folder failure")
            {
                Command::InferenceEvent(Event::LoopFailed(_, epoch, _))
                    if epoch == invalid_epoch =>
                {
                    break;
                }
                _ => {}
            }
        }
        std::fs::remove_dir_all(invalid_folder).expect("remove test folder");
        std::fs::remove_file(folder.join("sample.bmp")).expect("remove valid sample");
        std::fs::remove_file(folder.join("empty.bmp")).expect("remove empty sample");
        std::fs::remove_dir(folder).expect("remove loop test folder");
    }
}
