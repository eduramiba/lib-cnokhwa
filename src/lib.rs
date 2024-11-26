mod video_format;
mod video_device;

use dcv_color_primitives::{convert_image, ColorSpace, ImageFormat, PixelFormat};
use nokhwa::error::NokhwaError;

use nokhwa::pixel_format::RgbFormat;
use nokhwa::{native_api_backend, nokhwa_check, nokhwa_initialize, query, utils::{
    CameraFormat, CameraIndex,
    RequestedFormat, RequestedFormatType, Resolution,
}, Buffer, CallbackCamera, Camera};
use std::collections::{HashMap, HashSet};

use std::os::raw::c_char;
use std::ptr;

use crate::video_device::VideoDevice;
use crate::video_format::VideoFormat;
use nokhwa::utils::FrameFormat;
use parking_lot::Mutex;
use std::sync::{Arc, LazyLock, MutexGuard, PoisonError};

// This small library exposes nokhwa as a simple C library.
// Disclaimer: It's literally my first Rust program, so probably it will contain some bad parts!

static RESULT_OK : i32 = 0;
static RESULT_YES : i32 = RESULT_OK;
static RESULT_NO : i32 = -256;

static ERROR_DEVICE_NOT_FOUND : i32 = -1;
static ERROR_FORMAT_NOT_FOUND : i32 = -2;
static ERROR_OPENING_DEVICE : i32 = -3;
static ERROR_SESSION_ALREADY_STARTED : i32 = -4;
static ERROR_SESSION_NOT_STARTED : i32 = -5;
static ERROR_STATE_NOT_INITIALIZED : i32 = -6;
static ERROR_READING_CAMERA_SESSION : i32 = -7;
static ERROR_READING_FRAME : i32 = -8;
static ERROR_DECODING_FRAME : i32 = -9;
static ERROR_BUFFER_NULL : i32 = -10;
static ERROR_BUFFER_NOT_ENOUGH_CAPACITY : i32 = -11;
static ERROR_UNKNOWN : i32 = -512;

static STATUS_AUTHORIZED : i32 = 0;
static STATUS_DENIED : i32 = -1;


fn list_devices() -> Result<Vec<VideoDevice>, &'static str> {
    let backend = match native_api_backend() {
        Some(b) => b,
        None => return Err("Error creating native API backend"),
    };

    let Ok(devices) = query(backend) else { return Err("Error listing devices") };

    let mut result: Vec<VideoDevice> = vec![];
    for device in devices {
        let mut unique_formats: HashSet<VideoFormat> = HashSet::new();

        let index = device.index().clone();
        let requested_format = RequestedFormat::new::<RgbFormat>(RequestedFormatType::None);

        let mut camera = match Camera::with_backend(index, requested_format, backend.clone()) {
            Ok(cam) => cam,
            Err(_) => continue
        };

        let camera_formats = match camera.compatible_camera_formats() {
            Ok(f) => f,
            Err(_) => continue
        };

        for format in camera_formats {
            let vf = VideoFormat {
                width: format.resolution().width(),
                height: format.resolution().height(),
                format: format.format(),
                frame_rate: format.frame_rate()
            };

            unique_formats.insert(vf);
        }

        let mut formats: Vec<VideoFormat> = unique_formats.iter().cloned().collect();
        formats.sort();

        let model_id = device.description().to_string();
        let unique_id = if device.misc().is_empty() { device.description().to_string() } else { device.misc().to_string() };
        let name = device.human_name();

        result.push(VideoDevice {
            index: device.index().clone(),
            model_id,
            unique_id,
            name,
            formats
        });
    }

    Ok(result)
}

#[derive(Clone)]
struct Session {
    pub camera: Arc<Mutex<CallbackCamera>>
}

#[derive(Clone)]
struct State {
    pub devices: Vec<VideoDevice>,
    pub camera_sessions: HashMap<CameraIndex, Session>
}

impl State {
    pub fn current() -> Option<State> {
        let read_guard = STATE.lock();

        read_guard.clone()
    }
    pub fn make_current(self) -> Result<(), PoisonError<MutexGuard<'static, Option<State>>>> {
        let mut w = STATE.lock();
        *w = Some(self);

        Ok(())
    }
}

static STATE: LazyLock<Mutex<Option<State>>> = LazyLock::new(Default::default);

#[no_mangle]
pub extern "C" fn cnokhwa_initialize() -> i32 {
    match list_devices() {
        Ok(devices) => {
            {
                let current_state = State::current();
                let sessions = current_state.map(|state| { state.camera_sessions});

                let camera_sessions = sessions.unwrap_or(HashMap::new());

                let new_state = State { devices, camera_sessions };
                let result = new_state.make_current();

                match result {
                    Ok(()) => RESULT_OK,
                    Err(_) => {
                        eprintln!("Error setting up new state");
                        ERROR_UNKNOWN
                    }
                }
            }
        },
        Err(e) => {
            eprintln!("Error listing devices: {:?}", e);
            ERROR_UNKNOWN
        }
    }
}

#[no_mangle]
pub extern "C" fn cnokhwa_ask_videocapture_auth() {
    nokhwa_initialize(|_granted| {
        // NOOP
    });
}

#[no_mangle]
pub extern "C" fn cnokhwa_has_videocapture_auth() -> i32 {
    if nokhwa_check() { STATUS_AUTHORIZED } else { STATUS_DENIED }
}


#[no_mangle]
pub extern "C" fn cnokhwa_devices_count() -> i32 {
    let Some(state) = State::current() else { return ERROR_STATE_NOT_INITIALIZED };

    state.devices.len() as i32
}

#[no_mangle]
pub extern "C" fn cnokhwa_device_name(device_index: i32, buf: *mut c_char, buf_len: usize) -> usize {
    let Some(state) = State::current() else { return 0 };

    if device_index < 0 || (device_index as usize) >= state.devices.len() {
        return 0;
    }

    let name = &state.devices[device_index as usize].name;

    unsafe {
        copy_str(name, buf, buf_len)
    }
}

#[no_mangle]
pub extern "C" fn cnokhwa_device_unique_id(device_index: i32, buf: *mut c_char, buf_len: usize) -> usize {
    let Some(state) = State::current() else { return 0 };

    if device_index < 0 || (device_index as usize) >= state.devices.len() {
        return 0;
    }

    let unique_id = &state.devices[device_index as usize].unique_id;

    unsafe {
        copy_str(unique_id, buf, buf_len)
    }
}

#[no_mangle]
pub extern "C" fn cnokhwa_device_model_id(device_index: i32, buf: *mut c_char, buf_len: usize) -> usize {
    let Some(state) = State::current() else { return 0 };

    if device_index < 0 || (device_index as usize) >= state.devices.len() {
        return 0;
    }

    let model_id = &state.devices[device_index as usize].model_id;

    unsafe {
        copy_str(model_id, buf, buf_len)
    }
}

#[no_mangle]
pub extern "C" fn cnokhwa_device_formats_count(device_index: i32) -> i32 {
    let Some(state) = State::current() else { return ERROR_STATE_NOT_INITIALIZED };

    if device_index < 0 || (device_index as usize) >= state.devices.len() {
        return ERROR_DEVICE_NOT_FOUND;
    }

    let device = &state.devices[device_index as usize];

    device.formats.len() as i32
}

#[no_mangle]
pub extern "C" fn cnokhwa_device_format_width(device_index: i32, format_index: i32) -> u32 {
    let Some(state) = State::current() else { return 0 };

    if device_index < 0 || (device_index as usize) >= state.devices.len() {
        return 0;
    }

    let device = &state.devices[device_index as usize];

    if format_index < 0 || (format_index as usize) >= device.formats.len() {
        return 0;
    }

    device.formats[format_index as usize].width
}

#[no_mangle]
pub extern "C" fn cnokhwa_device_format_height(device_index: i32, format_index: i32) -> u32 {
    let Some(state) = State::current() else { return 0 };

    if device_index < 0 || (device_index as usize) >= state.devices.len() {
        return 0;
    }

    let device = &state.devices[device_index as usize];

    if format_index < 0 || (format_index as usize) >= device.formats.len() {
        return 0;
    }

    device.formats[format_index as usize].height
}

#[no_mangle]
pub extern "C" fn cnokhwa_device_format_frame_rate(device_index: i32, format_index: i32) -> u32 {
    let Some(state) = State::current() else { return 0 };

    if device_index < 0 || (device_index as usize) >= state.devices.len() {
        return 0;
    }

    let device = &state.devices[device_index as usize];

    if format_index < 0 || (format_index as usize) >= device.formats.len() {
        return 0;
    }

    device.formats[format_index as usize].frame_rate
}

#[no_mangle]
pub extern "C" fn cnokhwa_device_format_type(
    device_index: i32,
    format_index: i32,
    buf: *mut c_char,
    buf_len: usize,
) -> usize {
    let Some(state) = State::current() else { return 0 };

    if device_index < 0 || (device_index as usize) >= state.devices.len() {
        return 0;
    }

    let device = &state.devices[device_index as usize];

    if format_index < 0 || (format_index as usize) >= device.formats.len() {
        return 0;
    }

    let type_str = &device.formats[format_index as usize].format.to_string();

    unsafe {
        copy_str(type_str, buf, buf_len)
    }
}

#[no_mangle]
pub extern "C" fn cnokhwa_start_capture(device_index: u32, width: u32, height: u32) -> i32 {
    let mut state_guard = STATE.lock();
    let state = match state_guard.as_mut() {
        Some(s) => s,
        None => return ERROR_STATE_NOT_INITIALIZED
    };

    let device = match state.devices.get(device_index as usize) {
        Some(dev) => dev,
        None => return ERROR_DEVICE_NOT_FOUND
    };

    if state.camera_sessions.get(&device.index).is_some() {
        return ERROR_SESSION_ALREADY_STARTED;
    }

    fn format_priority(format: FrameFormat) -> u8 {
        match format {
            FrameFormat::RAWRGB => 4,
            FrameFormat::NV12 => 3,
            FrameFormat::YUYV => 2,
            FrameFormat::MJPEG => 1,
            _ => 0, // Unknown or other formats
        }
    }

    let Some(format) = device.formats.iter()
        .filter(|f| f.width == width && f.height == height)
        .max_by(|a, b| {
            let priority_a = format_priority(a.format);
            let priority_b = format_priority(b.format);

            if priority_a == priority_b {
                // If priorities are equal, compare frame rates
                a.frame_rate.cmp(&b.frame_rate)
            } else {
                // Otherwise, compare priorities
                priority_a.cmp(&priority_b)
            }
        })
    else { return ERROR_FORMAT_NOT_FOUND };

    println!("Starting capture on device {} ({}) with format {}", device.index, device.name, format.format);

    let resolution = Resolution::new(width, height);
    let camera_format = CameraFormat::new(resolution, format.format, format.frame_rate);

    let format = RequestedFormat::new::<RgbFormat>(RequestedFormatType::Exact(camera_format));

    let Ok(mut camera_session) = CallbackCamera::new(device.index.clone(), format, |_buffer| {
        //NOOP
    }) else { return ERROR_OPENING_DEVICE };

    let Ok(_) =  camera_session.open_stream() else { return ERROR_OPENING_DEVICE };

    // save camera session in state:
    let session = Session {
        camera: Arc::new(Mutex::new(camera_session))
    };
    state.camera_sessions.insert(device.index.clone(), session);

    RESULT_OK
}

#[no_mangle]
pub extern "C" fn cnokhwa_stop_capture(device_index: u32) -> i32 {
    let mut state_guard = STATE.lock();
    let state = match state_guard.as_mut() {
        Some(s) => s,
        None => return ERROR_STATE_NOT_INITIALIZED
    };

    let device = match state.devices.get(device_index as usize) {
        Some(dev) => dev,
        None => return ERROR_DEVICE_NOT_FOUND
    };

    let Some(session) = state.camera_sessions.remove(&device.index) else { return ERROR_SESSION_NOT_STARTED };

    let mut camera_session_guard = session.camera.lock();

    println!("Stopping capture on device {} ({})", device.index, device.name);

    let Ok(_) = camera_session_guard.stop_stream() else { return ERROR_SESSION_NOT_STARTED };

    RESULT_OK
}

#[no_mangle]
pub extern "C" fn cnokhwa_has_first_frame(device_index: u32) -> i32 {
    let mut state_guard = STATE.lock();
    let state = match state_guard.as_mut() {
        Some(s) => s,
        None => return ERROR_STATE_NOT_INITIALIZED
    };

    let device = match state.devices.get(device_index as usize) {
        Some(dev) => dev,
        None => return ERROR_DEVICE_NOT_FOUND
    };

    let Some(session) = state.camera_sessions.get(&device.index)
    else { return ERROR_SESSION_NOT_STARTED };

    let mut camera_session_guard = session.camera.lock();

    match camera_session_guard.poll_frame() {
        Ok(_) => RESULT_YES,
        Err(_) => RESULT_NO
    }
}

#[no_mangle]
pub extern "C" fn cnokhwa_grab_frame(
    device_index: u32,
    buffer: *mut u8,
    available_bytes: usize,
) -> i32 {
    let frame = {
        let mut state_guard = STATE.lock();
        let state = match state_guard.as_mut() {
            Some(s) => s,
            None => return ERROR_STATE_NOT_INITIALIZED,
        };

        let device = match state.devices.get(device_index as usize) {
            Some(dev) => dev,
            None => return ERROR_DEVICE_NOT_FOUND,
        };

        let Some(session) = state.camera_sessions.get(&device.index)
        else { return ERROR_SESSION_NOT_STARTED };

        let camera_session_guard = session.camera.lock();

        match camera_session_guard.last_frame() {
            Ok(f) => f,
            Err(_) => return ERROR_READING_FRAME,
        }
    };

    let resolution = frame.resolution();
    let width = resolution.width() as usize;
    let height = resolution.height() as usize;
    let dst_size = width * height * 3; // RGB output

    if available_bytes < dst_size {
        return ERROR_BUFFER_NOT_ENOUGH_CAPACITY;
    }

    unsafe {
        if buffer.is_null() {
            return ERROR_BUFFER_NULL;
        }

        // Create a mutable slice from the raw pointer
        let output = std::slice::from_raw_parts_mut(buffer, dst_size);

        match convert_to_rgb(frame, output) {
            Ok(_) => RESULT_OK,
            Err(e) => {
                eprintln!("Decoding error: {:?}", e);
                ERROR_DECODING_FRAME
            }
        }
    }
}


#[no_mangle]
pub extern "C" fn cnokhwa_frame_width(device_index: u32) -> i32 {
    let mut state_guard = STATE.lock();
    let state = match state_guard.as_mut() {
        Some(s) => s,
        None => return ERROR_STATE_NOT_INITIALIZED
    };

    let device = match state.devices.get(device_index as usize) {
        Some(dev) => dev,
        None => return ERROR_DEVICE_NOT_FOUND
    };

    let Some(session) = state.camera_sessions.get(&device.index)
    else { return ERROR_SESSION_NOT_STARTED };

    let camera_session_guard = session.camera.lock();

    match camera_session_guard.camera_format() {
        Ok(f) => f.width() as i32,
        Err(_) => ERROR_READING_CAMERA_SESSION
    }
}

#[no_mangle]
pub extern "C" fn cnokhwa_frame_height(device_index: u32) -> i32 {
    let mut state_guard = STATE.lock();
    let state = match state_guard.as_mut() {
        Some(s) => s,
        None => return ERROR_STATE_NOT_INITIALIZED
    };

    let device = match state.devices.get(device_index as usize) {
        Some(dev) => dev,
        None => return ERROR_DEVICE_NOT_FOUND
    };

    let Some(session) = state.camera_sessions.get(&device.index)
    else { return ERROR_SESSION_NOT_STARTED };

    let camera_session_guard = session.camera.lock();

    match camera_session_guard.camera_format() {
        Ok(f) => f.height() as i32,
        Err(_) => ERROR_READING_CAMERA_SESSION
    }
}

#[no_mangle]
pub extern "C" fn cnokhwa_frame_bytes_per_row(device_index: u32) -> i32 {
    let mut state_guard = STATE.lock();
    let state = match state_guard.as_mut() {
        Some(s) => s,
        None => return ERROR_STATE_NOT_INITIALIZED
    };

    let device = match state.devices.get(device_index as usize) {
        Some(dev) => dev,
        None => return ERROR_DEVICE_NOT_FOUND
    };

    let Some(session) = state.camera_sessions.get(&device.index)
    else { return ERROR_SESSION_NOT_STARTED };

    let camera_session_guard = session.camera.lock();

    match camera_session_guard.camera_format() {
        Ok(f) => (f.width() as i32) * 3, // RGB
        Err(_) => ERROR_READING_CAMERA_SESSION,
    }
}

/// Copies a Rust string into a C buffer, similar to `strncpy` in C.
///
/// # Arguments
///
/// * `s` - The Rust string slice to copy.
/// * `buf` - A mutable pointer to the destination buffer.
/// * `length` - The maximum number of bytes to copy.
///
/// # Safety
///
/// This function is unsafe because it involves raw pointer manipulation.
/// Ensure that `buf` is valid and has at least `length` bytes allocated.
unsafe fn copy_str(s: &str, buf: *mut c_char, length: usize) -> usize {
    if length == 0 {
        return 0;
    }

    // Convert the Rust string to bytes (UTF-8)
    let bytes = s.as_bytes();
    let len_to_copy = std::cmp::min(bytes.len(), length - 1); // Reserve space for null terminator

    // Copy the bytes into the destination buffer
    ptr::copy_nonoverlapping(bytes.as_ptr(), buf as *mut u8, len_to_copy);

    // Null-terminate the string
    *buf.add(len_to_copy) = 0;

    len_to_copy
}

fn convert_to_rgb(frame: Buffer, output: &mut [u8]) -> Result<(), NokhwaError> {
    let buffer = frame.buffer();

    match frame.source_frame_format() {
        FrameFormat::NV12 => convert_to_rgb_with_dcv(
            buffer,
            frame.source_frame_format(),
            frame.resolution(),
            output,
        ),
        _ => {
            match frame.decode_image_to_buffer::<RgbFormat>(output) {
                Ok(_) => {
                    Ok(())
                }
                Err(e) => Err(e),
            }
        }
    }
}

// DCV has faster implementations but only works for NV12 to RGB
fn convert_to_rgb_with_dcv(
    buffer: &[u8],
    frame_format: FrameFormat,
    resolution: Resolution,
    output: &mut [u8],
) -> Result<(), NokhwaError> {
    let width = resolution.width();
    let height = resolution.height();
    let width_usize = width as usize;
    let height_usize = height as usize;

    let dst_size = width_usize * height_usize * 3; // RGB output

    if output.len() != dst_size {
        return Err(NokhwaError::ProcessFrameError {
            src: frame_format,
            destination: "RGB".to_string(),
            error: format!(
                "Output buffer size mismatch: expected {}, got {}",
                dst_size,
                output.len()
            ),
        });
    }

    match frame_format {
        FrameFormat::NV12 => {
            let src_format = ImageFormat {
                pixel_format: PixelFormat::Nv12,
                color_space: ColorSpace::Bt601,
                num_planes: 1,
            };
            let dst_format = ImageFormat {
                pixel_format: PixelFormat::Rgb,
                color_space: ColorSpace::Rgb,
                num_planes: 1,
            };

            convert_image(
                width,
                height,
                &src_format,
                None,
                &[buffer],
                &dst_format,
                None,
                &mut [&mut output[..]],
            )
                .map_err(|e| NokhwaError::ProcessFrameError {
                    src: frame_format,
                    destination: "RGB".to_string(),
                    error: format!("Conversion error: {:?}", e),
                })?;
            Ok(())
        }
        _ => Err(NokhwaError::NotImplementedError(format!(
            "Unsupported frame format for dcv conversion: {:?}",
            frame_format
        ))),
    }
}