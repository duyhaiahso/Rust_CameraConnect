//! Focused bindings for the installed pylon C v10 ABI. Keep signatures aligned
//! with `Development/include/pylonc/PylonC.h` from the selected SDK.

use std::ffi::{c_char, c_void};

pub type Handle = *mut c_void;
pub const OK: i32 = 0;
pub const ACCESS_CONTROL_STREAM: i32 = 0x01 | 0x02;
pub const GRABBED: i32 = 2;
pub const PIXEL_RGB8: i32 = 0x0218_0014;
pub const IMAGE_FORMAT_PNG: i32 = 3;
pub const ORIENTATION_TOP_DOWN: i32 = 0;
pub const NODE_INTEGER: i32 = 0;
pub const NODE_BOOLEAN: i32 = 1;
pub const NODE_FLOAT: i32 = 2;
pub const NODE_COMMAND: i32 = 3;
pub const NODE_STRING: i32 = 4;
pub const NODE_ENUMERATION: i32 = 5;
pub const NODE_ENUM_ENTRY: i32 = 6;
pub const NODE_CATEGORY: i32 = 7;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct GrabResult {
    pub context: *const c_void,
    pub buffer_handle: Handle,
    pub buffer: *const c_void,
    pub status: i32,
    pub payload_type: i32,
    pub pixel_type: i32,
    pub timestamp: u64,
    pub width: i32,
    pub height: i32,
    pub offset_x: i32,
    pub offset_y: i32,
    pub padding_x: i32,
    pub padding_y: i32,
    pub payload_size: u64,
    pub error_code: u32,
    pub block_id: u64,
}

impl Default for GrabResult {
    fn default() -> Self {
        // C writes every field before a successful retrieve.
        unsafe { std::mem::zeroed() }
    }
}

#[link(name = "PylonC_v10")]
unsafe extern "system" {
    pub fn PylonInitialize() -> i32;
    pub fn PylonTerminate(shut_down_logging: u8) -> i32;
    pub fn PylonEnumerateDevices(count: *mut usize) -> i32;
    pub fn PylonGigEEnumerateAllDevices(count: *mut usize) -> i32;
    pub fn PylonGigEForceIp(
        mac: *const c_char,
        ip: *const c_char,
        mask: *const c_char,
        gateway: *const c_char,
    ) -> i32;
    pub fn PylonGigERestartIpConfiguration(mac: *const c_char) -> i32;
    pub fn PylonGigEBroadcastIpConfiguration(
        mac: *const c_char,
        persistent: u8,
        dhcp: u8,
        ip: *const c_char,
        mask: *const c_char,
        gateway: *const c_char,
        user_name: *const c_char,
        applied: *mut u8,
    ) -> i32;
    pub fn PylonGetDeviceInfoHandle(index: usize, info: *mut Handle) -> i32;
    pub fn PylonDeviceInfoGetPropertyValueByName(
        info: Handle,
        name: *const c_char,
        value: *mut c_char,
        buffer_len: *mut usize,
    ) -> i32;
    pub fn PylonDeviceInfoGetNumProperties(info: Handle, count: *mut usize) -> i32;
    pub fn PylonDeviceInfoGetPropertyName(
        info: Handle,
        index: usize,
        name: *mut c_char,
        buffer_len: *mut usize,
    ) -> i32;
    pub fn PylonCreateDeviceByIndex(index: usize, device: *mut Handle) -> i32;
    pub fn PylonDeviceOpen(device: Handle, access_mode: i32) -> i32;
    pub fn PylonDeviceClose(device: Handle) -> i32;
    pub fn PylonDestroyDevice(device: Handle) -> i32;
    pub fn PylonDeviceGetDeviceInfoHandle(device: Handle, info: *mut Handle) -> i32;
    pub fn PylonDeviceGetNodeMap(device: Handle, map: *mut Handle) -> i32;
    pub fn PylonDeviceGetTLNodeMap(device: Handle, map: *mut Handle) -> i32;
    pub fn PylonDeviceFeatureIsAvailable(device: Handle, name: *const c_char) -> u8;
    pub fn PylonDeviceFeatureToString(
        device: Handle,
        name: *const c_char,
        buffer: *mut c_char,
        buffer_len: *mut usize,
    ) -> i32;
    pub fn PylonDeviceFeatureFromString(
        device: Handle,
        name: *const c_char,
        value: *const c_char,
    ) -> i32;
    pub fn PylonDeviceExecuteCommandFeature(device: Handle, name: *const c_char) -> i32;
    pub fn PylonDeviceGetNumStreamGrabberChannels(device: Handle, count: *mut usize) -> i32;
    pub fn PylonDeviceGetStreamGrabber(device: Handle, index: usize, grabber: *mut Handle) -> i32;
    pub fn PylonStreamGrabberOpen(grabber: Handle) -> i32;
    pub fn PylonStreamGrabberClose(grabber: Handle) -> i32;
    pub fn PylonStreamGrabberGetNodeMap(grabber: Handle, map: *mut Handle) -> i32;
    pub fn PylonStreamGrabberGetWaitObject(grabber: Handle, wait: *mut Handle) -> i32;
    pub fn PylonStreamGrabberGetPayloadSize(
        device: Handle,
        grabber: Handle,
        payload_size: *mut usize,
    ) -> i32;
    pub fn PylonStreamGrabberSetMaxNumBuffer(grabber: Handle, count: usize) -> i32;
    pub fn PylonStreamGrabberSetMaxBufferSize(grabber: Handle, size: usize) -> i32;
    pub fn PylonStreamGrabberPrepareGrab(grabber: Handle) -> i32;
    pub fn PylonStreamGrabberRegisterBuffer(
        grabber: Handle,
        buffer: *mut c_void,
        len: usize,
        buffer_handle: *mut Handle,
    ) -> i32;
    pub fn PylonStreamGrabberQueueBuffer(
        grabber: Handle,
        buffer_handle: Handle,
        context: *const c_void,
    ) -> i32;
    pub fn PylonStreamGrabberStartStreamingIfMandatory(grabber: Handle) -> i32;
    pub fn PylonStreamGrabberStopStreamingIfMandatory(grabber: Handle) -> i32;
    pub fn PylonStreamGrabberFlushBuffersToOutput(grabber: Handle) -> i32;
    pub fn PylonStreamGrabberRetrieveResult(
        grabber: Handle,
        result: *mut GrabResult,
        ready: *mut u8,
    ) -> i32;
    pub fn PylonStreamGrabberDeregisterBuffer(grabber: Handle, buffer_handle: Handle) -> i32;
    pub fn PylonStreamGrabberFinishGrab(grabber: Handle) -> i32;
    pub fn PylonWaitObjectWait(wait: Handle, timeout_ms: u32, ready: *mut u8) -> i32;
    pub fn PylonImageFormatConverterCreate(converter: *mut Handle) -> i32;
    pub fn PylonImageFormatConverterSetOutputPixelFormat(converter: Handle, pixel_type: i32)
    -> i32;
    pub fn PylonImageFormatConverterSetOutputPaddingX(converter: Handle, padding: usize) -> i32;
    pub fn PylonImageFormatConverterGetBufferSizeForConversion(
        converter: Handle,
        source_type: i32,
        width: u32,
        height: u32,
        size: *mut usize,
    ) -> i32;
    pub fn PylonImageFormatConverterConvert(
        converter: Handle,
        target: *mut c_void,
        target_size: usize,
        source: *const c_void,
        source_size: usize,
        source_type: i32,
        width: u32,
        height: u32,
        padding_x: usize,
        orientation: i32,
    ) -> i32;
    pub fn PylonImageFormatConverterDestroy(converter: Handle) -> i32;
    pub fn PylonImagePersistenceSave(
        format: i32,
        path: *const c_char,
        buffer: *const c_void,
        buffer_size: usize,
        pixel_type: i32,
        width: u32,
        height: u32,
        padding_x: usize,
        orientation: i32,
        options: *mut c_void,
    ) -> i32;

    pub fn GenApiNodeMapGetNode(map: Handle, name: *const c_char, node: *mut Handle) -> i32;
    pub fn GenApiNodeMapGetNumNodes(map: Handle, count: *mut usize) -> i32;
    pub fn GenApiNodeMapGetNodeByIndex(map: Handle, index: usize, node: *mut Handle) -> i32;
    pub fn GenApiNodeGetName(node: Handle, name: *mut c_char, len: *mut usize) -> i32;
    pub fn GenApiNodeGetDisplayName(node: Handle, name: *mut c_char, len: *mut usize) -> i32;
    pub fn GenApiNodeGetDescription(node: Handle, text: *mut c_char, len: *mut usize) -> i32;
    pub fn GenApiNodeGetToolTip(node: Handle, text: *mut c_char, len: *mut usize) -> i32;
    pub fn GenApiNodeGetType(node: Handle, node_type: *mut i32) -> i32;
    pub fn GenApiNodeGetAccessMode(node: Handle, mode: *mut i32) -> i32;
    pub fn GenApiNodeGetVisibility(node: Handle, visibility: *mut i32) -> i32;
    pub fn GenApiNodeIsReadable(node: Handle, readable: *mut u8) -> i32;
    pub fn GenApiNodeIsWritable(node: Handle, writable: *mut u8) -> i32;
    pub fn GenApiNodeIsAvailable(node: Handle, available: *mut u8) -> i32;
    pub fn GenApiNodeToString(node: Handle, text: *mut c_char, len: *mut usize) -> i32;
    pub fn GenApiNodeFromString(node: Handle, text: *const c_char) -> i32;
    pub fn GenApiCategoryGetNumFeatures(node: Handle, count: *mut usize) -> i32;
    pub fn GenApiCategoryGetFeatureByIndex(node: Handle, index: usize, child: *mut Handle) -> i32;
    pub fn GenApiIntegerGetMin(node: Handle, min: *mut i64) -> i32;
    pub fn GenApiIntegerGetMax(node: Handle, max: *mut i64) -> i32;
    pub fn GenApiIntegerGetInc(node: Handle, increment: *mut i64) -> i32;
    pub fn GenApiBooleanSetValue(node: Handle, value: u8) -> i32;
    pub fn GenApiFloatGetMin(node: Handle, min: *mut f64) -> i32;
    pub fn GenApiFloatGetMax(node: Handle, max: *mut f64) -> i32;
    pub fn GenApiFloatGetUnit(node: Handle, unit: *mut c_char, len: *mut usize) -> i32;
    pub fn GenApiEnumerationGetNumEntries(node: Handle, count: *mut usize) -> i32;
    pub fn GenApiEnumerationGetEntryByIndex(node: Handle, index: usize, entry: *mut Handle) -> i32;
    pub fn GenApiEnumerationEntryGetSymbolic(
        node: Handle,
        value: *mut c_char,
        len: *mut usize,
    ) -> i32;
}
