//! The native bindings expose format discovery before a capture format is selected.
#[cfg(not(target_os = "linux"))]
use nokhwa::{
    error::NokhwaError,
    utils::{CameraFormat, CameraIndex},
};

#[cfg(target_os = "linux")]
pub(crate) use nokhwa_bindings_linux::{
    active_frame_rate, probe_capture_formats as supported_formats,
};

#[cfg(target_os = "windows")]
pub(crate) fn supported_formats(index: &CameraIndex) -> Result<Vec<CameraFormat>, NokhwaError> {
    nokhwa_bindings_windows::wmf::MediaFoundationDevice::new(index.clone())?
        .compatible_format_list()
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
pub(crate) fn supported_formats(index: &CameraIndex) -> Result<Vec<CameraFormat>, NokhwaError> {
    nokhwa_bindings_macos::AVCaptureDevice::new(index)?.supported_formats()
}

#[cfg(not(any(
    target_os = "linux",
    target_os = "windows",
    target_os = "macos",
    target_os = "ios"
)))]
pub(crate) fn supported_formats(_: &CameraIndex) -> Result<Vec<CameraFormat>, NokhwaError> {
    Err(NokhwaError::UnsupportedOperationError(
        nokhwa::utils::ApiBackend::Auto,
    ))
}

#[cfg(not(target_os = "linux"))]
pub(crate) fn active_frame_rate(_: &CameraIndex) -> Result<f64, NokhwaError> {
    // Nokhwa 0.10.11 exposes the requested rate or a format's maximum on
    // AVFoundation. Its Media Foundation binding reads only the denominator
    // of MF_MT_FRAME_RATE. Neither identifies the negotiated frame interval.
    Ok(0.0)
}
