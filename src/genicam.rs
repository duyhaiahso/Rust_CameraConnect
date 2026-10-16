//! GenICam inspection lives on the camera worker thread. Handles are borrowed
//! from an open pylon device/stream and never cross into the Slint UI thread.

use crate::camera::{Device, Stream};
use crate::pylon_ffi as ffi;
use crate::roi::Limits;
use std::collections::HashSet;
use std::ffi::{CString, c_char};
use std::ptr;

type Result<T> = std::result::Result<T, String>;

fn check(operation: &str, code: i32) -> Result<()> {
    if code == ffi::OK {
        Ok(())
    } else {
        Err(format!("{operation} failed (pylon 0x{:08X})", code as u32))
    }
}

fn read_text(
    operation: &str,
    mut call: impl FnMut(*mut c_char, *mut usize) -> i32,
) -> Result<String> {
    let mut buffer = vec![0u8; 256];
    loop {
        let mut length = buffer.len();
        let code = call(buffer.as_mut_ptr().cast::<c_char>(), &mut length);
        if code == ffi::OK {
            let end = buffer
                .iter()
                .position(|byte| *byte == 0)
                .unwrap_or(length.min(buffer.len()));
            return Ok(String::from_utf8_lossy(&buffer[..end]).into_owned());
        }
        if length > buffer.len() && length <= 1024 * 1024 {
            buffer.resize(length, 0);
        } else {
            return Err(format!("{operation} failed (pylon 0x{:08X})", code as u32));
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum MapKind {
    Device,
    Transport,
    Stream,
}

impl MapKind {
    fn key(self) -> &'static str {
        match self {
            Self::Device => "device",
            Self::Transport => "transport",
            Self::Stream => "stream",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Device => "Device",
            Self::Transport => "Transport",
            Self::Stream => "Stream",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct FeatureKey {
    pub map: MapKind,
    pub name: String,
}

impl FeatureKey {
    pub fn id(&self) -> String {
        format!("{}/{}", self.map.key(), self.name)
    }
}

#[derive(Clone)]
pub struct FeatureNode {
    pub key: FeatureKey,
    pub display: String,
    pub depth: usize,
    pub ancestors: Vec<String>,
    pub category: bool,
    pub access: String,
    pub visibility: String,
}

pub struct FeatureTree {
    pub nodes: Vec<FeatureNode>,
    pub warnings: Vec<String>,
}

pub struct FeatureDetail {
    pub key: FeatureKey,
    pub display: String,
    pub description: String,
    pub node_type: String,
    pub visibility: String,
    pub access: String,
    pub value: String,
    pub limits: String,
    pub options: Vec<String>,
    pub editable: bool,
}

fn map_handle(device: &Device, stream: Option<&Stream>, kind: MapKind) -> Result<ffi::Handle> {
    let mut map = ptr::null_mut();
    let code = match kind {
        MapKind::Device => unsafe { ffi::PylonDeviceGetNodeMap(device.raw_handle(), &mut map) },
        MapKind::Transport => unsafe {
            ffi::PylonDeviceGetTLNodeMap(device.raw_handle(), &mut map)
        },
        MapKind::Stream => {
            let stream = stream.ok_or("Stream map is available while live view is running")?;
            unsafe { ffi::PylonStreamGrabberGetNodeMap(stream.raw_handle(), &mut map) }
        }
    };
    check("Get GenICam node map", code)?;
    if map.is_null() {
        return Err("pylon returned an empty GenICam node map".to_owned());
    }
    Ok(map)
}

fn node_handle(map: ffi::Handle, name: &str) -> Result<ffi::Handle> {
    let name = CString::new(name).map_err(|_| "Invalid GenICam node name".to_owned())?;
    let mut node = ptr::null_mut();
    check("Get GenICam node", unsafe {
        ffi::GenApiNodeMapGetNode(map, name.as_ptr(), &mut node)
    })?;
    if node.is_null() {
        return Err("GenICam node is unavailable".to_owned());
    }
    Ok(node)
}

fn node_name(node: ffi::Handle) -> Result<String> {
    read_text("Read GenICam name", |buffer, len| unsafe {
        ffi::GenApiNodeGetName(node, buffer, len)
    })
}

fn node_type(node: ffi::Handle) -> Result<i32> {
    let mut kind = -1;
    check("Read GenICam type", unsafe {
        ffi::GenApiNodeGetType(node, &mut kind)
    })?;
    Ok(kind)
}

fn node_access(node: ffi::Handle) -> String {
    let mut mode = -1;
    if unsafe { ffi::GenApiNodeGetAccessMode(node, &mut mode) } != ffi::OK {
        return "Unknown".to_owned();
    }
    match mode {
        0 => "Not implemented",
        1 => "Unavailable",
        2 => "Write only",
        3 => "Read only",
        4 => "Read/write",
        _ => "Unknown",
    }
    .to_owned()
}

fn node_visibility(node: ffi::Handle) -> String {
    let mut visibility = -1;
    if unsafe { ffi::GenApiNodeGetVisibility(node, &mut visibility) } != ffi::OK {
        return "Unknown".to_owned();
    }
    match visibility {
        0 => "Beginner",
        1 => "Expert",
        2 => "Guru",
        3 => "Invisible",
        _ => "Unknown",
    }
    .to_owned()
}

fn is_readable(node: ffi::Handle) -> bool {
    let mut readable = 0;
    unsafe { ffi::GenApiNodeIsReadable(node, &mut readable) == ffi::OK && readable != 0 }
}

fn is_writable(node: ffi::Handle) -> bool {
    let mut writable = 0;
    unsafe { ffi::GenApiNodeIsWritable(node, &mut writable) == ffi::OK && writable != 0 }
}

fn is_available(node: ffi::Handle) -> bool {
    let mut available = 0;
    unsafe { ffi::GenApiNodeIsAvailable(node, &mut available) == ffi::OK && available != 0 }
}

fn value(node: ffi::Handle) -> Result<String> {
    read_text("Read GenICam value", |buffer, len| unsafe {
        ffi::GenApiNodeToString(node, buffer, len)
    })
}

fn display_value(kind: i32, raw: String) -> String {
    if kind == ffi::NODE_BOOLEAN {
        match raw.to_ascii_lowercase().as_str() {
            "0" | "false" => "False".to_owned(),
            "1" | "true" => "True".to_owned(),
            _ => raw,
        }
    } else {
        raw
    }
}

fn display_name(node: ffi::Handle, name: &str) -> String {
    read_text("Read GenICam display name", |buffer, len| unsafe {
        ffi::GenApiNodeGetDisplayName(node, buffer, len)
    })
    .ok()
    .filter(|text| !text.is_empty())
    .unwrap_or_else(|| name.to_owned())
}

fn add_node(
    node: ffi::Handle,
    map: MapKind,
    depth: usize,
    ancestors: &[String],
    visited: &mut HashSet<String>,
    output: &mut Vec<FeatureNode>,
) -> Result<()> {
    let name = node_name(node)?;
    let key = FeatureKey { map, name };
    if !visited.insert(key.id()) {
        return Ok(());
    }
    let kind = node_type(node)?;
    let category = kind == ffi::NODE_CATEGORY;
    output.push(FeatureNode {
        display: display_name(node, &key.name),
        key: key.clone(),
        depth,
        ancestors: ancestors.to_vec(),
        category,
        access: node_access(node),
        visibility: node_visibility(node),
    });
    if category && depth < 32 {
        let mut count = 0;
        check("Read category children", unsafe {
            ffi::GenApiCategoryGetNumFeatures(node, &mut count)
        })?;
        let mut child_ancestors = ancestors.to_vec();
        child_ancestors.push(key.id());
        for index in 0..count {
            let mut child = ptr::null_mut();
            if unsafe { ffi::GenApiCategoryGetFeatureByIndex(node, index, &mut child) } == ffi::OK
                && !child.is_null()
            {
                let _ = add_node(child, map, depth + 1, &child_ancestors, visited, output);
            }
        }
    }
    Ok(())
}

pub fn inspect(device: &Device, stream: Option<&Stream>) -> FeatureTree {
    let mut nodes = Vec::new();
    let mut warnings = Vec::new();
    for kind in [MapKind::Device, MapKind::Transport, MapKind::Stream] {
        if kind == MapKind::Stream && stream.is_none() {
            continue;
        }
        let map = match map_handle(device, stream, kind) {
            Ok(map) => map,
            Err(error) => {
                warnings.push(format!("{} map: {error}", kind.label()));
                continue;
            }
        };
        let mut visited = HashSet::new();
        if let Ok(root) = node_handle(map, "Root")
            && let Err(error) = add_node(root, kind, 0, &[], &mut visited, &mut nodes)
        {
            warnings.push(format!("{} root: {error}", kind.label()));
        }
        let mut count = 0;
        if let Err(error) = check("Count GenICam nodes", unsafe {
            ffi::GenApiNodeMapGetNumNodes(map, &mut count)
        }) {
            warnings.push(format!("{} map: {error}", kind.label()));
            continue;
        }
        let mut skipped = 0;
        for index in 0..count {
            let mut node = ptr::null_mut();
            if unsafe { ffi::GenApiNodeMapGetNodeByIndex(map, index, &mut node) } != ffi::OK
                || node.is_null()
                || add_node(node, kind, 1, &[], &mut visited, &mut nodes).is_err()
            {
                skipped += 1;
            }
        }
        if skipped > 0 {
            warnings.push(format!(
                "{} map: {skipped} node(s) could not be read",
                kind.label()
            ));
        }
    }
    FeatureTree { nodes, warnings }
}

pub fn device_properties(device: &Device) -> Result<Vec<(String, String)>> {
    let mut info = ptr::null_mut();
    check("Get device information", unsafe {
        ffi::PylonDeviceGetDeviceInfoHandle(device.raw_handle(), &mut info)
    })?;
    let mut count = 0;
    check("Count device properties", unsafe {
        ffi::PylonDeviceInfoGetNumProperties(info, &mut count)
    })?;
    let mut properties = Vec::with_capacity(count);
    for index in 0..count {
        let name = read_text("Read device property name", |buffer, len| unsafe {
            ffi::PylonDeviceInfoGetPropertyName(info, index, buffer, len)
        })?;
        let c_name = CString::new(name.as_str()).map_err(|_| "Invalid property name".to_owned())?;
        let property = read_text("Read device property", |buffer, len| unsafe {
            ffi::PylonDeviceInfoGetPropertyValueByName(info, c_name.as_ptr(), buffer, len)
        })?;
        properties.push((name, property));
    }
    Ok(properties)
}

const SAFE_RUNTIME_NODES: &[&str] = &[
    "ExposureTime",
    "ExposureTimeAbs",
    "ExposureTimeRaw",
    "ExposureAuto",
    "Gain",
    "GainAbs",
    "GainRaw",
    "GainAuto",
    "AcquisitionFrameRate",
    "AcquisitionFrameRateAbs",
    "AcquisitionFrameRateEnable",
    "Width",
    "Height",
    "OffsetX",
    "OffsetY",
    "PixelFormat",
    "ReverseX",
    "ReverseY",
];

fn safe_runtime_node(key: &FeatureKey, kind: i32) -> bool {
    key.map == MapKind::Device
        && SAFE_RUNTIME_NODES.contains(&key.name.as_str())
        && matches!(
            kind,
            ffi::NODE_INTEGER | ffi::NODE_FLOAT | ffi::NODE_BOOLEAN | ffi::NODE_ENUMERATION
        )
}

fn enum_options(node: ffi::Handle) -> Vec<String> {
    let mut count = 0;
    if unsafe { ffi::GenApiEnumerationGetNumEntries(node, &mut count) } != ffi::OK {
        return Vec::new();
    }
    let mut options = Vec::new();
    for index in 0..count {
        let mut entry = ptr::null_mut();
        if unsafe { ffi::GenApiEnumerationGetEntryByIndex(node, index, &mut entry) } != ffi::OK
            || entry.is_null()
            || !is_available(entry)
        {
            continue;
        }
        if let Ok(symbol) = read_text("Read enum option", |buffer, len| unsafe {
            ffi::GenApiEnumerationEntryGetSymbolic(entry, buffer, len)
        }) {
            options.push(symbol);
        }
    }
    options
}

fn limits(node: ffi::Handle, kind: i32) -> String {
    match kind {
        ffi::NODE_INTEGER => {
            let (mut min, mut max, mut increment) = (0i64, 0i64, 0i64);
            let min_ok = unsafe { ffi::GenApiIntegerGetMin(node, &mut min) } == ffi::OK;
            let max_ok = unsafe { ffi::GenApiIntegerGetMax(node, &mut max) } == ffi::OK;
            let inc_ok = unsafe { ffi::GenApiIntegerGetInc(node, &mut increment) } == ffi::OK;
            if min_ok && max_ok {
                if inc_ok {
                    format!("{min} .. {max} (step {increment})")
                } else {
                    format!("{min} .. {max}")
                }
            } else {
                String::new()
            }
        }
        ffi::NODE_FLOAT => {
            let (mut min, mut max) = (0.0f64, 0.0f64);
            let min_ok = unsafe { ffi::GenApiFloatGetMin(node, &mut min) } == ffi::OK;
            let max_ok = unsafe { ffi::GenApiFloatGetMax(node, &mut max) } == ffi::OK;
            let unit = read_text("Read unit", |buffer, len| unsafe {
                ffi::GenApiFloatGetUnit(node, buffer, len)
            })
            .unwrap_or_default();
            if min_ok && max_ok {
                format!("{min} .. {max} {unit}").trim().to_owned()
            } else {
                unit
            }
        }
        _ => String::new(),
    }
}

pub fn detail(device: &Device, stream: Option<&Stream>, key: &FeatureKey) -> Result<FeatureDetail> {
    let map = map_handle(device, stream, key.map)?;
    let node = node_handle(map, &key.name)?;
    let kind = node_type(node)?;
    let readable = is_readable(node);
    let available = is_available(node);
    let options = match kind {
        ffi::NODE_ENUMERATION => enum_options(node),
        ffi::NODE_BOOLEAN => vec!["False".to_owned(), "True".to_owned()],
        _ => Vec::new(),
    };
    let type_label = match kind {
        ffi::NODE_INTEGER => "Integer",
        ffi::NODE_BOOLEAN => "Boolean",
        ffi::NODE_FLOAT => "Float",
        ffi::NODE_COMMAND => "Command",
        ffi::NODE_STRING => "String",
        ffi::NODE_ENUMERATION => "Enumeration",
        ffi::NODE_ENUM_ENTRY => "Enum entry",
        ffi::NODE_CATEGORY => "Category",
        _ => "Other",
    };
    let mut description = read_text("Read node description", |buffer, len| unsafe {
        ffi::GenApiNodeGetDescription(node, buffer, len)
    })
    .unwrap_or_default();
    if description.is_empty() {
        description = read_text("Read node tooltip", |buffer, len| unsafe {
            ffi::GenApiNodeGetToolTip(node, buffer, len)
        })
        .unwrap_or_default();
    }
    Ok(FeatureDetail {
        key: key.clone(),
        display: display_name(node, &key.name),
        description,
        node_type: type_label.to_owned(),
        visibility: node_visibility(node),
        access: node_access(node),
        value: if available
            && readable
            && matches!(
                kind,
                ffi::NODE_INTEGER
                    | ffi::NODE_FLOAT
                    | ffi::NODE_BOOLEAN
                    | ffi::NODE_STRING
                    | ffi::NODE_ENUMERATION
            ) {
            value(node)
                .map(|raw| display_value(kind, raw))
                .unwrap_or_else(|error| format!("<read error: {error}>"))
        } else {
            "—".to_owned()
        },
        limits: if available {
            limits(node, kind)
        } else {
            String::new()
        },
        options,
        editable: available && readable && is_writable(node) && safe_runtime_node(key, kind),
    })
}

pub fn configured_fps(device: &Device) -> Option<String> {
    let map = map_handle(device, None, MapKind::Device).ok()?;
    for name in [
        "BslResultingAcquisitionFrameRate",
        "ResultingFrameRate",
        "ResultingFrameRateAbs",
        "ResultingAcquisitionFrameRate",
        "AcquisitionFrameRate",
        "AcquisitionFrameRateAbs",
    ] {
        if let Ok(node) = node_handle(map, name)
            && is_available(node)
            && is_readable(node)
            && let Ok(fps) = value(node)
        {
            return Some(fps);
        }
    }
    None
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RoiState {
    pub width: i64,
    pub height: i64,
    pub offset_x: i64,
    pub offset_y: i64,
}

#[derive(Clone, Copy, Debug)]
pub struct RoiInfo {
    pub state: RoiState,
    pub width: Limits,
    pub height: Limits,
}

fn roi_key(name: &str) -> FeatureKey {
    FeatureKey {
        map: MapKind::Device,
        name: name.to_owned(),
    }
}

fn roi_integer(device: &Device, name: &str) -> Result<i64> {
    let map = map_handle(device, None, MapKind::Device)?;
    let node = node_handle(map, name)?;
    if !is_available(node) || !is_readable(node) || node_type(node)? != ffi::NODE_INTEGER {
        return Err(format!("{name} is not a readable integer camera feature"));
    }
    value(node)?
        .parse()
        .map_err(|_| format!("Invalid {name} value"))
}

fn roi_limits(device: &Device, name: &str) -> Result<Limits> {
    let map = map_handle(device, None, MapKind::Device)?;
    let node = node_handle(map, name)?;
    if !is_available(node) || !is_readable(node) || node_type(node)? != ffi::NODE_INTEGER {
        return Err(format!("{name} has no integer limits"));
    }
    let (mut min, mut max, mut step) = (0, 0, 1);
    check("Read ROI minimum", unsafe {
        ffi::GenApiIntegerGetMin(node, &mut min)
    })?;
    check("Read ROI maximum", unsafe {
        ffi::GenApiIntegerGetMax(node, &mut max)
    })?;
    let _ = unsafe { ffi::GenApiIntegerGetInc(node, &mut step) };
    Ok(Limits {
        min,
        max,
        step: step.max(1),
    })
}

pub fn roi_info(device: &Device) -> Result<RoiInfo> {
    let state = RoiState {
        width: roi_integer(device, "Width")?,
        height: roi_integer(device, "Height")?,
        offset_x: roi_integer(device, "OffsetX")?,
        offset_y: roi_integer(device, "OffsetY")?,
    };
    let mut width = roi_limits(device, "Width")?;
    let mut height = roi_limits(device, "Height")?;
    let full_width = roi_integer(device, "WidthMax")?;
    let full_height = roi_integer(device, "HeightMax")?;
    width.max = full_width;
    height.max = full_height;
    Ok(RoiInfo {
        state,
        width,
        height,
    })
}

fn write_roi_integer(device: &Device, name: &str, target: i64) -> Result<()> {
    if roi_integer(device, name)? == target {
        return Ok(());
    }
    apply(device, &roi_key(name), &target.to_string())?;
    Ok(())
}

fn write_roi(device: &Device, target: RoiState, center: bool) -> Result<RoiState> {
    // Clear offsets before growing either dimension. Recompute offset limits
    // after dimensions change because many cameras expose dynamic maxima.
    let neutral_x = roi_limits(device, "OffsetX")?.min;
    let neutral_y = roi_limits(device, "OffsetY")?.min;
    write_roi_integer(device, "OffsetX", neutral_x)?;
    write_roi_integer(device, "OffsetY", neutral_y)?;
    write_roi_integer(device, "Width", target.width)?;
    write_roi_integer(device, "Height", target.height)?;
    let (offset_x, offset_y) = if center {
        let x_limits = roi_limits(device, "OffsetX")?;
        let y_limits = roi_limits(device, "OffsetY")?;
        let x = x_limits
            .nearest(x_limits.min + (x_limits.max - x_limits.min) / 2)
            .ok_or("OffsetX cannot be centered")?;
        let y = y_limits
            .nearest(y_limits.min + (y_limits.max - y_limits.min) / 2)
            .ok_or("OffsetY cannot be centered")?;
        (x, y)
    } else {
        (target.offset_x, target.offset_y)
    };
    write_roi_integer(device, "OffsetX", offset_x)?;
    write_roi_integer(device, "OffsetY", offset_y)?;
    let actual = roi_info(device)?.state;
    if actual.width != target.width
        || actual.height != target.height
        || actual.offset_x != offset_x
        || actual.offset_y != offset_y
    {
        return Err(format!("Camera ROI readback differs: {actual:?}"));
    }
    Ok(actual)
}

pub fn apply_roi(device: &Device, target: RoiState, center: bool) -> Result<(RoiState, RoiState)> {
    let before = roi_info(device)?.state;
    match write_roi(device, target, center) {
        Ok(after) => Ok((before, after)),
        Err(error) => {
            let recovery = write_roi(device, before, false)
                .err()
                .map(|failure| format!("; ROI rollback failed: {failure}"))
                .unwrap_or_default();
            Err(format!("ROI change failed: {error}{recovery}"))
        }
    }
}

pub fn restore_roi(device: &Device, original: RoiState) -> Result<()> {
    write_roi(device, original, false).map(|_| ())
}

pub fn apply(device: &Device, key: &FeatureKey, proposed: &str) -> Result<(String, String)> {
    let map = map_handle(device, None, key.map)?;
    let node = node_handle(map, &key.name)?;
    let kind = node_type(node)?;
    if !safe_runtime_node(key, kind)
        || !is_available(node)
        || !is_readable(node)
        || !is_writable(node)
    {
        return Err("This node is not an available, writable runtime control".to_owned());
    }
    let proposed = proposed.trim();
    if proposed.is_empty() {
        return Err("Enter a value".to_owned());
    }
    let original = value(node)?;
    match kind {
        ffi::NODE_INTEGER => {
            let number: i64 = proposed
                .parse()
                .map_err(|_| "Enter an integer".to_owned())?;
            let (mut min, mut max, mut increment) = (0i64, 0i64, 0i64);
            check("Read minimum", unsafe {
                ffi::GenApiIntegerGetMin(node, &mut min)
            })?;
            check("Read maximum", unsafe {
                ffi::GenApiIntegerGetMax(node, &mut max)
            })?;
            if number < min || number > max {
                return Err(format!("Value must be from {min} to {max}"));
            }
            if unsafe { ffi::GenApiIntegerGetInc(node, &mut increment) } == ffi::OK
                && increment > 0
                && (number - min) % increment != 0
            {
                return Err(format!("Value must use step {increment} from {min}"));
            }
        }
        ffi::NODE_FLOAT => {
            let number: f64 = proposed
                .parse()
                .map_err(|_| "Enter a decimal number".to_owned())?;
            let (mut min, mut max) = (0.0f64, 0.0f64);
            check("Read minimum", unsafe {
                ffi::GenApiFloatGetMin(node, &mut min)
            })?;
            check("Read maximum", unsafe {
                ffi::GenApiFloatGetMax(node, &mut max)
            })?;
            if !number.is_finite() || number < min || number > max {
                return Err(format!("Value must be from {min} to {max}"));
            }
        }
        ffi::NODE_ENUMERATION => {
            if !enum_options(node).iter().any(|option| option == proposed) {
                return Err("Choose an available option".to_owned());
            }
        }
        ffi::NODE_BOOLEAN => {
            if !proposed.eq_ignore_ascii_case("true") && !proposed.eq_ignore_ascii_case("false") {
                return Err("Choose True or False".to_owned());
            }
        }
        _ => return Err("Unsupported editor type".to_owned()),
    }
    if kind == ffi::NODE_BOOLEAN {
        check("Set GenICam boolean", unsafe {
            ffi::GenApiBooleanSetValue(node, u8::from(proposed.eq_ignore_ascii_case("true")))
        })?;
    } else {
        let proposed = CString::new(proposed).map_err(|_| "Invalid value".to_owned())?;
        check("Set GenICam value", unsafe {
            ffi::GenApiNodeFromString(node, proposed.as_ptr())
        })?;
    }
    match value(node) {
        Ok(after) => Ok((original, after)),
        Err(error) => {
            let recovery = restore(device, key, &original)
                .err()
                .map(|failure| format!("; recovery also failed: {failure}"))
                .unwrap_or_default();
            Err(format!(
                "Could not read back the new value: {error}{recovery}"
            ))
        }
    }
}

pub fn restore(device: &Device, key: &FeatureKey, original: &str) -> Result<()> {
    let map = map_handle(device, None, key.map)?;
    let node = node_handle(map, &key.name)?;
    if !is_available(node) || !is_writable(node) {
        return Err("Node is no longer writable".to_owned());
    }
    if node_type(node)? == ffi::NODE_BOOLEAN {
        let enabled = matches!(original.to_ascii_lowercase().as_str(), "1" | "true");
        return check("Restore GenICam boolean", unsafe {
            ffi::GenApiBooleanSetValue(node, u8::from(enabled))
        });
    }
    let original = CString::new(original).map_err(|_| "Invalid stored value".to_owned())?;
    check("Restore GenICam value", unsafe {
        ffi::GenApiNodeFromString(node, original.as_ptr())
    })
}
