use crate::pylon_ffi as ffi;
use std::ffi::{CStr, CString, c_char, c_void};
use std::path::Path;
use std::ptr;

type Result<T> = std::result::Result<T, String>;

fn check(operation: &str, code: i32) -> Result<()> {
    if code == ffi::OK {
        Ok(())
    } else {
        Err(format!("{operation} failed (pylon 0x{:08X})", code as u32))
    }
}

#[derive(Clone)]
pub struct CameraInfo {
    pub key: String,
    pub label: String,
    pub device_class: String,
    pub serial: String,
    pub gige: Option<GigEInfo>,
    pub reachable: bool,
}

#[derive(Clone)]
pub struct GigEInfo {
    pub mac: String,
    pub ip: String,
    pub mask: String,
    pub gateway: String,
    pub interface_ip: String,
    pub user_name: String,
}

pub struct Runtime;

impl Runtime {
    pub fn new() -> Result<Self> {
        check("PylonInitialize", unsafe { ffi::PylonInitialize() })?;
        Ok(Self)
    }

    pub fn enumerate(&self) -> Result<Vec<CameraInfo>> {
        let mut count = 0;
        check("PylonEnumerateDevices", unsafe {
            ffi::PylonEnumerateDevices(&mut count)
        })?;
        let mut cameras = Vec::with_capacity(count);
        for index in 0..count {
            let mut info = ptr::null_mut();
            check("PylonGetDeviceInfoHandle", unsafe {
                ffi::PylonGetDeviceInfoHandle(index, &mut info)
            })?;
            cameras.push(read_camera_info(info, true)?);
        }
        Ok(cameras)
    }

    /// Keep the normal enumeration for USB and connectable GigE devices, then
    /// add GigE cameras that pylon sees only through the all-subnets scan.
    pub fn enumerate_with_gige(&self) -> Result<(Vec<CameraInfo>, Option<String>)> {
        let mut cameras = self.enumerate()?;
        let mut count = 0;
        let code = unsafe { ffi::PylonGigEEnumerateAllDevices(&mut count) };
        if code != ffi::OK {
            return Ok((
                cameras,
                Some(format!(
                    "GigE all-subnets scan failed (pylon 0x{:08X})",
                    code as u32
                )),
            ));
        }
        for index in 0..count {
            let mut info = ptr::null_mut();
            check("PylonGetDeviceInfoHandle", unsafe {
                ffi::PylonGetDeviceInfoHandle(index, &mut info)
            })?;
            let found = read_camera_info(info, false)?;
            let found_mac = found
                .gige
                .as_ref()
                .map(|gige| gige.mac.as_str())
                .unwrap_or("");
            if let Some(existing) = cameras.iter_mut().find(|camera| {
                (!found_mac.is_empty()
                    && camera
                        .gige
                        .as_ref()
                        .is_some_and(|gige| gige.mac == found_mac))
                    || (!found.serial.is_empty()
                        && camera.serial == found.serial
                        && camera.device_class == found.device_class)
            }) {
                existing.gige = found.gige;
            } else {
                cameras.push(found);
            }
        }
        for camera in &mut cameras {
            if let Some(gige) = &camera.gige {
                camera.label = format!(
                    "{}  ·  {}  ·  {}{}",
                    camera.label,
                    gige.ip,
                    gige.mac,
                    if camera.reachable {
                        ""
                    } else {
                        "  ·  WRONG SUBNET"
                    }
                );
            }
        }
        Ok((cameras, None))
    }

    pub fn configure_gige_ip(
        &self,
        mac: &str,
        ip: &str,
        mask: &str,
        gateway: &str,
        user_name: &str,
        persistent: bool,
    ) -> Result<()> {
        let normalized_mac: String = mac.chars().filter(|ch| ch.is_ascii_hexdigit()).collect();
        if normalized_mac.len() != 12 {
            return Err("Invalid camera MAC address".to_owned());
        }
        let mac = CString::new(normalized_mac).map_err(|_| "Invalid camera MAC address")?;
        let ip = CString::new(ip).map_err(|_| "Invalid IP address")?;
        let mask = CString::new(mask).map_err(|_| "Invalid subnet mask")?;
        let gateway = CString::new(gateway).map_err(|_| "Invalid gateway")?;
        if persistent {
            let user_name = CString::new(user_name).map_err(|_| "Invalid camera name")?;
            let mut applied = 0u8;
            check("PylonGigEBroadcastIpConfiguration", unsafe {
                ffi::PylonGigEBroadcastIpConfiguration(
                    mac.as_ptr(),
                    1,
                    0,
                    ip.as_ptr(),
                    mask.as_ptr(),
                    gateway.as_ptr(),
                    user_name.as_ptr(),
                    &mut applied,
                )
            })?;
            if applied == 0 {
                return Err("Camera did not accept the persistent IP configuration".to_owned());
            }
            // BroadcastIpConfiguration stores the static settings but may leave
            // a previously forced address active. Basler's Utility_IpConfig
            // sample restarts the IP configuration cycle after a successful write.
            let restart_code = unsafe { ffi::PylonGigERestartIpConfiguration(mac.as_ptr()) };
            if restart_code != ffi::OK {
                return Err(format!(
                    "Persistent IP was accepted, but restarting camera IP configuration failed (pylon 0x{:08X}). The settings may take effect after a camera power cycle.",
                    restart_code as u32
                ));
            }
        } else {
            check("PylonGigEForceIp", unsafe {
                ffi::PylonGigEForceIp(mac.as_ptr(), ip.as_ptr(), mask.as_ptr(), gateway.as_ptr())
            })?;
        }
        Ok(())
    }

    pub fn connect(&self, key: &str) -> Result<Device> {
        // Enumeration indices are only valid until the next enumeration.
        let cameras = self.enumerate()?;
        let index = cameras
            .iter()
            .position(|camera| camera.key == key)
            .ok_or_else(|| {
                "Selected camera is no longer available. Refresh the list.".to_owned()
            })?;
        let mut handle = ptr::null_mut();
        check("PylonCreateDeviceByIndex", unsafe {
            ffi::PylonCreateDeviceByIndex(index, &mut handle)
        })?;
        let mut device = Device {
            handle,
            opened: false,
            original_acquisition: None,
            original_selector: None,
            changed_triggers: Vec::new(),
        };
        check("PylonDeviceOpen", unsafe {
            ffi::PylonDeviceOpen(handle, ffi::ACCESS_CONTROL_STREAM)
        })?;
        device.opened = true;
        device.configure_free_run()?;
        Ok(device)
    }
}

fn read_camera_info(info: ffi::Handle, reachable: bool) -> Result<CameraInfo> {
    let key = property(info, c"FullName")?;
    let name = property(info, c"FriendlyName").unwrap_or_default();
    let model = property(info, c"ModelName").unwrap_or_default();
    let serial = property(info, c"SerialNumber").unwrap_or_default();
    let class = property(info, c"DeviceClass").unwrap_or_default();
    let display = if name.is_empty() { model } else { name };
    let mac = property(info, c"MacAddress").unwrap_or_default();
    let gige = if mac.is_empty() {
        None
    } else {
        Some(GigEInfo {
            mac,
            ip: property(info, c"IpAddress").unwrap_or_default(),
            mask: property(info, c"SubnetMask").unwrap_or_default(),
            gateway: property(info, c"DefaultGateway").unwrap_or_default(),
            interface_ip: property(info, c"Interface").unwrap_or_default(),
            user_name: property(info, c"UserDefinedName").unwrap_or_default(),
        })
    };
    Ok(CameraInfo {
        key,
        label: format!("{display}  ·  {serial}  ·  {class}"),
        device_class: class,
        serial,
        gige,
        reachable,
    })
}

impl Drop for Runtime {
    fn drop(&mut self) {
        unsafe { ffi::PylonTerminate(1) };
    }
}

fn property(info: ffi::Handle, name: &CStr) -> Result<String> {
    let mut buffer = [0 as c_char; 1024];
    let mut length = buffer.len();
    check("PylonDeviceInfoGetPropertyValueByName", unsafe {
        ffi::PylonDeviceInfoGetPropertyValueByName(
            info,
            name.as_ptr(),
            buffer.as_mut_ptr(),
            &mut length,
        )
    })?;
    Ok(unsafe { CStr::from_ptr(buffer.as_ptr()) }
        .to_string_lossy()
        .into_owned())
}

pub struct Device {
    handle: ffi::Handle,
    opened: bool,
    original_acquisition: Option<String>,
    original_selector: Option<String>,
    changed_triggers: Vec<(String, String)>,
}

impl Device {
    pub(crate) fn raw_handle(&self) -> ffi::Handle {
        self.handle
    }

    fn get_feature(&self, name: &CStr) -> Result<String> {
        let mut buffer = [0 as c_char; 256];
        let mut length = buffer.len();
        check(&format!("Read {}", name.to_string_lossy()), unsafe {
            ffi::PylonDeviceFeatureToString(
                self.handle,
                name.as_ptr(),
                buffer.as_mut_ptr(),
                &mut length,
            )
        })?;
        Ok(unsafe { CStr::from_ptr(buffer.as_ptr()) }
            .to_string_lossy()
            .into_owned())
    }

    fn set_feature(&self, name: &CStr, value: &str) -> Result<()> {
        let value = CString::new(value).map_err(|_| "Invalid feature value".to_owned())?;
        check(&format!("Set {}", name.to_string_lossy()), unsafe {
            ffi::PylonDeviceFeatureFromString(self.handle, name.as_ptr(), value.as_ptr())
        })
    }

    fn configure_free_run(&mut self) -> Result<()> {
        self.original_acquisition = Some(self.get_feature(c"AcquisitionMode")?);
        let selector_available = unsafe {
            ffi::PylonDeviceFeatureIsAvailable(self.handle, c"TriggerSelector".as_ptr()) != 0
        };
        if selector_available {
            self.original_selector = Some(self.get_feature(c"TriggerSelector")?);
            for selector in ["AcquisitionStart", "FrameBurstStart", "FrameStart"] {
                let entry = CString::new(format!("EnumEntry_TriggerSelector_{selector}"))
                    .expect("static feature name");
                if unsafe { ffi::PylonDeviceFeatureIsAvailable(self.handle, entry.as_ptr()) } == 0 {
                    continue;
                }
                self.set_feature(c"TriggerSelector", selector)?;
                let previous = self.get_feature(c"TriggerMode")?;
                if previous != "Off" {
                    // Record before changing so a partial configuration is restored on error.
                    self.changed_triggers.push((selector.to_owned(), previous));
                    self.set_feature(c"TriggerMode", "Off")?;
                }
            }
            if let Some(original) = &self.original_selector {
                self.set_feature(c"TriggerSelector", original)?;
            }
        }
        self.set_feature(c"AcquisitionMode", "Continuous")?;
        Ok(())
    }

    fn restore(&mut self) -> Vec<String> {
        let mut errors = Vec::new();
        for (selector, mode) in &self.changed_triggers {
            if let Err(error) = self.set_feature(c"TriggerSelector", selector) {
                errors.push(error);
                continue;
            }
            if let Err(error) = self.set_feature(c"TriggerMode", mode) {
                errors.push(error);
            }
        }
        if let Some(original) = &self.original_acquisition
            && let Err(error) = self.set_feature(c"AcquisitionMode", original)
        {
            errors.push(error);
        }
        if let Some(original) = &self.original_selector
            && let Err(error) = self.set_feature(c"TriggerSelector", original)
        {
            errors.push(error);
        }
        self.changed_triggers.clear();
        self.original_acquisition = None;
        self.original_selector = None;
        errors
    }

    pub fn close(&mut self) -> Vec<String> {
        if self.handle.is_null() {
            return Vec::new();
        }
        let mut errors = if self.opened {
            self.restore()
        } else {
            Vec::new()
        };
        if self.opened
            && let Err(error) = check("PylonDeviceClose", unsafe {
                ffi::PylonDeviceClose(self.handle)
            })
        {
            errors.push(error);
        }
        if let Err(error) = check("PylonDestroyDevice", unsafe {
            ffi::PylonDestroyDevice(self.handle)
        }) {
            errors.push(error);
        }
        self.opened = false;
        self.handle = ptr::null_mut();
        errors
    }
}

impl Drop for Device {
    fn drop(&mut self) {
        let _ = self.close();
    }
}

pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub rgb: Vec<u8>,
}

impl Frame {
    pub fn save_png(&self, path: &Path) -> Result<()> {
        let filename = path
            .to_str()
            .ok_or_else(|| "The image path is not valid Unicode".to_owned())?;
        let filename = CString::new(filename)
            .map_err(|_| "The image path contains an invalid NUL character".to_owned())?;
        check("PylonImagePersistenceSave", unsafe {
            ffi::PylonImagePersistenceSave(
                ffi::IMAGE_FORMAT_PNG,
                filename.as_ptr(),
                self.rgb.as_ptr().cast::<c_void>(),
                self.rgb.len(),
                ffi::PIXEL_RGB8,
                self.width,
                self.height,
                0,
                ffi::ORIENTATION_TOP_DOWN,
                ptr::null_mut(),
            )
        })
    }
}

pub struct Converter(ffi::Handle);

impl Converter {
    pub fn new() -> Result<Self> {
        let mut handle = ptr::null_mut();
        check("PylonImageFormatConverterCreate", unsafe {
            ffi::PylonImageFormatConverterCreate(&mut handle)
        })?;
        let converter = Self(handle);
        check("Set RGB8 output", unsafe {
            ffi::PylonImageFormatConverterSetOutputPixelFormat(handle, ffi::PIXEL_RGB8)
        })?;
        check("Set output padding", unsafe {
            ffi::PylonImageFormatConverterSetOutputPaddingX(handle, 0)
        })?;
        Ok(converter)
    }

    fn convert(&self, result: &ffi::GrabResult) -> Result<Frame> {
        let width = u32::try_from(result.width).map_err(|_| "Invalid frame width".to_owned())?;
        let height = u32::try_from(result.height).map_err(|_| "Invalid frame height".to_owned())?;
        if width == 0 || height == 0 || result.buffer.is_null() || result.padding_x < 0 {
            return Err("Invalid image payload from camera".to_owned());
        }
        let expected = usize::try_from(width)
            .ok()
            .and_then(|w| usize::try_from(height).ok().and_then(|h| w.checked_mul(h)))
            .and_then(|px| px.checked_mul(3))
            .ok_or_else(|| "Frame dimensions are too large".to_owned())?;
        let mut size = 0;
        check("Get converted image size", unsafe {
            ffi::PylonImageFormatConverterGetBufferSizeForConversion(
                self.0,
                result.pixel_type,
                width,
                height,
                &mut size,
            )
        })?;
        if size != expected {
            return Err(format!(
                "Unexpected converted frame size: {size} bytes, expected {expected}"
            ));
        }
        let source_size = usize::try_from(result.payload_size)
            .map_err(|_| "Camera payload is too large".to_owned())?;
        let mut rgb = vec![0u8; size];
        check("Convert image to RGB8", unsafe {
            ffi::PylonImageFormatConverterConvert(
                self.0,
                rgb.as_mut_ptr().cast::<c_void>(),
                rgb.len(),
                result.buffer,
                source_size,
                result.pixel_type,
                width,
                height,
                result.padding_x as usize,
                ffi::ORIENTATION_TOP_DOWN,
            )
        })?;
        Ok(Frame { width, height, rgb })
    }
}

impl Drop for Converter {
    fn drop(&mut self) {
        unsafe { ffi::PylonImageFormatConverterDestroy(self.0) };
    }
}

struct RegisteredBuffer {
    bytes: Vec<u8>,
    handle: ffi::Handle,
}

pub struct Stream {
    device_handle: ffi::Handle,
    handle: ffi::Handle,
    wait: ffi::Handle,
    opened: bool,
    prepared: bool,
    streaming: bool,
    acquiring: bool,
    received_frames: u64,
    buffers: Vec<RegisteredBuffer>,
}

impl Stream {
    pub(crate) fn raw_handle(&self) -> ffi::Handle {
        self.handle
    }

    pub fn start(device: &Device) -> Result<Self> {
        let mut channels = 0;
        check("Get image streams", unsafe {
            ffi::PylonDeviceGetNumStreamGrabberChannels(device.handle, &mut channels)
        })?;
        if channels == 0 {
            return Err("The selected device does not provide a pylon image stream".to_owned());
        }
        let mut handle = ptr::null_mut();
        check("Get stream grabber", unsafe {
            ffi::PylonDeviceGetStreamGrabber(device.handle, 0, &mut handle)
        })?;
        let mut stream = Self {
            device_handle: device.handle,
            handle,
            wait: ptr::null_mut(),
            opened: false,
            prepared: false,
            streaming: false,
            acquiring: false,
            received_frames: 0,
            buffers: Vec::new(),
        };
        check("Open stream grabber", unsafe {
            ffi::PylonStreamGrabberOpen(handle)
        })?;
        stream.opened = true;
        check("Get stream wait object", unsafe {
            ffi::PylonStreamGrabberGetWaitObject(handle, &mut stream.wait)
        })?;
        let mut payload_size = 0;
        check("Get payload size", unsafe {
            ffi::PylonStreamGrabberGetPayloadSize(device.handle, handle, &mut payload_size)
        })?;
        if payload_size == 0 {
            return Err("Camera reports an empty image payload".to_owned());
        }
        if payload_size > 256 * 1024 * 1024 {
            return Err(format!(
                "Camera payload of {payload_size} bytes exceeds the 256 MiB test-app limit"
            ));
        }
        const BUFFER_COUNT: usize = 4;
        check("Set buffer count", unsafe {
            ffi::PylonStreamGrabberSetMaxNumBuffer(handle, BUFFER_COUNT)
        })?;
        check("Set buffer size", unsafe {
            ffi::PylonStreamGrabberSetMaxBufferSize(handle, payload_size)
        })?;
        check("Prepare stream", unsafe {
            ffi::PylonStreamGrabberPrepareGrab(handle)
        })?;
        stream.prepared = true;
        for _ in 0..BUFFER_COUNT {
            let mut bytes = vec![0u8; payload_size];
            let mut buffer_handle = ptr::null_mut();
            check("Register stream buffer", unsafe {
                ffi::PylonStreamGrabberRegisterBuffer(
                    handle,
                    bytes.as_mut_ptr().cast::<c_void>(),
                    bytes.len(),
                    &mut buffer_handle,
                )
            })?;
            stream.buffers.push(RegisteredBuffer {
                bytes,
                handle: buffer_handle,
            });
        }
        for buffer in &stream.buffers {
            check("Queue stream buffer", unsafe {
                ffi::PylonStreamGrabberQueueBuffer(handle, buffer.handle, ptr::null())
            })?;
        }
        check("Start stream", unsafe {
            ffi::PylonStreamGrabberStartStreamingIfMandatory(handle)
        })?;
        stream.streaming = true;
        // Attempt AcquisitionStop in Drop even if AcquisitionStart reports an error.
        stream.acquiring = true;
        check("Start acquisition", unsafe {
            ffi::PylonDeviceExecuteCommandFeature(device.handle, c"AcquisitionStart".as_ptr())
        })?;
        Ok(stream)
    }

    pub fn next_frame(&mut self, converter: &Converter, convert: bool) -> Result<Option<Frame>> {
        let mut ready = 0;
        check("Wait for frame", unsafe {
            ffi::PylonWaitObjectWait(self.wait, 80, &mut ready)
        })?;
        if ready == 0 {
            return Ok(None);
        }
        let mut latest = self
            .retrieve_frame()?
            .ok_or_else(|| "Stream signaled without a grab result".to_owned())?;

        // The low-level output queue is FIFO. After expensive RGB conversion or a
        // UI pause, discard older completed frames before choosing one to display.
        // Bound the drain to the registered buffer count so commands still run.
        for _ in 1..self.buffers.len() {
            let newer = match self.retrieve_frame() {
                Ok(Some(result)) => result,
                Ok(None) => break,
                Err(error) => {
                    let _ = self.requeue_frame(latest.buffer_handle);
                    return Err(error);
                }
            };
            if let Err(error) = self.requeue_frame(latest.buffer_handle) {
                let _ = self.requeue_frame(newer.buffer_handle);
                return Err(error);
            }
            latest = newer;
        }

        let frame = if convert {
            converter.convert(&latest).map(Some)
        } else {
            Ok(None)
        };
        self.requeue_frame(latest.buffer_handle)?;
        frame
    }

    fn retrieve_frame(&mut self) -> Result<Option<ffi::GrabResult>> {
        let mut result = ffi::GrabResult::default();
        let mut ready = 0;
        check("Retrieve frame", unsafe {
            ffi::PylonStreamGrabberRetrieveResult(self.handle, &mut result, &mut ready)
        })?;
        if ready == 0 {
            return Ok(None);
        }
        if result.status != ffi::GRABBED {
            let error = format!(
                "Camera frame failed (status {}, code 0x{:08X})",
                result.status, result.error_code
            );
            self.requeue_frame(result.buffer_handle)?;
            return Err(error);
        }
        self.received_frames += 1;
        Ok(Some(result))
    }

    fn requeue_frame(&self, handle: ffi::Handle) -> Result<()> {
        check("Requeue stream buffer", unsafe {
            ffi::PylonStreamGrabberQueueBuffer(self.handle, handle, ptr::null())
        })
    }

    pub fn received_frames(&self) -> u64 {
        self.received_frames
    }

    pub fn stop(&mut self) -> Vec<String> {
        let mut errors = Vec::new();
        if self.acquiring {
            if let Err(error) = check("Stop acquisition", unsafe {
                ffi::PylonDeviceExecuteCommandFeature(
                    self.device_handle,
                    c"AcquisitionStop".as_ptr(),
                )
            }) {
                errors.push(error);
            }
            self.acquiring = false;
        }
        errors.extend(self.release_stream());
        errors
    }

    fn release_stream(&mut self) -> Vec<String> {
        let mut errors = Vec::new();
        if self.streaming {
            if let Err(error) = check("Stop stream", unsafe {
                ffi::PylonStreamGrabberStopStreamingIfMandatory(self.handle)
            }) {
                errors.push(error);
            }
            self.streaming = false;
        }
        if self.prepared {
            if let Err(error) = check("Flush stream", unsafe {
                ffi::PylonStreamGrabberFlushBuffersToOutput(self.handle)
            }) {
                errors.push(error);
            }
            loop {
                let mut result = ffi::GrabResult::default();
                let mut ready = 0;
                let outcome = check("Drain stream", unsafe {
                    ffi::PylonStreamGrabberRetrieveResult(self.handle, &mut result, &mut ready)
                });
                if let Err(error) = outcome {
                    errors.push(error);
                    break;
                }
                if ready == 0 {
                    break;
                }
            }
            for buffer in self.buffers.drain(..) {
                if let Err(error) = check("Deregister buffer", unsafe {
                    ffi::PylonStreamGrabberDeregisterBuffer(self.handle, buffer.handle)
                }) {
                    errors.push(error);
                }
                drop(buffer.bytes);
            }
            if let Err(error) = check("Finish grab", unsafe {
                ffi::PylonStreamGrabberFinishGrab(self.handle)
            }) {
                errors.push(error);
            }
            self.prepared = false;
        }
        if self.opened {
            if let Err(error) = check("Close stream grabber", unsafe {
                ffi::PylonStreamGrabberClose(self.handle)
            }) {
                errors.push(error);
            }
            self.opened = false;
        }
        errors
    }
}

impl Drop for Stream {
    fn drop(&mut self) {
        // This also handles errors during acquisition or partial startup.
        let _ = self.stop();
    }
}
