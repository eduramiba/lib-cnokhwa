/*
 * Copyright 2022 l1npengtul <l1npengtul@protonmail.com> / The Nokhwa Contributors
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 *
 * Modified for lib-cnokhwa: read-only mode discovery and V4L2 format negotiation.
 */

#[cfg(target_os = "linux")]
mod internal {
    use nokhwa_core::{
        buffer::Buffer,
        error::NokhwaError,
        traits::CaptureBackendTrait,
        types::{
            ApiBackend, CameraControl, CameraFormat, CameraIndex, CameraInfo,
            ControlValueDescription, ControlValueSetter, FrameFormat, KnownCameraControl,
            KnownCameraControlFlag, RequestedFormat, RequestedFormatType, Resolution,
        },
    };
    use std::{
        borrow::Cow,
        collections::HashMap,
        io::{self, ErrorKind},
    };
    use v4l::v4l_sys::{
        V4L2_CID_BACKLIGHT_COMPENSATION, V4L2_CID_BRIGHTNESS, V4L2_CID_CONTRAST, V4L2_CID_EXPOSURE,
        V4L2_CID_FOCUS_RELATIVE, V4L2_CID_GAIN, V4L2_CID_GAMMA, V4L2_CID_HUE,
        V4L2_CID_IRIS_RELATIVE, V4L2_CID_PAN_RELATIVE, V4L2_CID_SATURATION, V4L2_CID_SHARPNESS,
        V4L2_CID_TILT_RELATIVE, V4L2_CID_WHITE_BALANCE_TEMPERATURE, V4L2_CID_ZOOM_RELATIVE,
    };
    use v4l::{
        control::{Control, Flags, Type, Value},
        frameinterval::FrameIntervalEnum,
        framesize::FrameSizeEnum,
        io::traits::CaptureStream,
        prelude::MmapStream,
        video::{capture::Parameters, Capture},
        Device, Format, FourCC,
    };

    #[cfg(feature = "no-arena-buffer")]
    use v4l::io::traits::Stream;

    /// Attempts to convert a [`KnownCameraControl`] into a V4L2 Control ID.
    /// If the associated control is not found, this will return `None` (`ColorEnable`, `Roll`)
    #[allow(clippy::cast_possible_truncation)]
    pub fn known_camera_control_to_id(ctrl: KnownCameraControl) -> u32 {
        match ctrl {
            KnownCameraControl::Brightness => V4L2_CID_BRIGHTNESS,
            KnownCameraControl::Contrast => V4L2_CID_CONTRAST,
            KnownCameraControl::Hue => V4L2_CID_HUE,
            KnownCameraControl::Saturation => V4L2_CID_SATURATION,
            KnownCameraControl::Sharpness => V4L2_CID_SHARPNESS,
            KnownCameraControl::Gamma => V4L2_CID_GAMMA,
            KnownCameraControl::WhiteBalance => V4L2_CID_WHITE_BALANCE_TEMPERATURE,
            KnownCameraControl::BacklightComp => V4L2_CID_BACKLIGHT_COMPENSATION,
            KnownCameraControl::Gain => V4L2_CID_GAIN,
            KnownCameraControl::Pan => V4L2_CID_PAN_RELATIVE,
            KnownCameraControl::Tilt => V4L2_CID_TILT_RELATIVE,
            KnownCameraControl::Zoom => V4L2_CID_ZOOM_RELATIVE,
            KnownCameraControl::Exposure => V4L2_CID_EXPOSURE,
            KnownCameraControl::Iris => V4L2_CID_IRIS_RELATIVE,
            KnownCameraControl::Focus => V4L2_CID_FOCUS_RELATIVE,
            KnownCameraControl::Other(id) => id as u32,
        }
    }

    /// Attempts to convert a [`u32`] V4L2 Control ID into a [`KnownCameraControl`]
    /// If the associated control is not found, this will return `None` (`ColorEnable`, `Roll`)
    #[allow(clippy::cast_lossless)]
    pub fn id_to_known_camera_control(id: u32) -> KnownCameraControl {
        match id {
            V4L2_CID_BRIGHTNESS => KnownCameraControl::Brightness,
            V4L2_CID_CONTRAST => KnownCameraControl::Contrast,
            V4L2_CID_HUE => KnownCameraControl::Hue,
            V4L2_CID_SATURATION => KnownCameraControl::Saturation,
            V4L2_CID_SHARPNESS => KnownCameraControl::Sharpness,
            V4L2_CID_GAMMA => KnownCameraControl::Gamma,
            V4L2_CID_WHITE_BALANCE_TEMPERATURE => KnownCameraControl::WhiteBalance,
            V4L2_CID_BACKLIGHT_COMPENSATION => KnownCameraControl::BacklightComp,
            V4L2_CID_GAIN => KnownCameraControl::Gain,
            V4L2_CID_PAN_RELATIVE => KnownCameraControl::Pan,
            V4L2_CID_TILT_RELATIVE => KnownCameraControl::Tilt,
            V4L2_CID_ZOOM_RELATIVE => KnownCameraControl::Zoom,
            V4L2_CID_EXPOSURE => KnownCameraControl::Exposure,
            V4L2_CID_IRIS_RELATIVE => KnownCameraControl::Iris,
            V4L2_CID_FOCUS_RELATIVE => KnownCameraControl::Focus,
            id => KnownCameraControl::Other(id as u128),
        }
    }

    /// query v4l2 cameras
    #[allow(clippy::unnecessary_wraps)]
    #[allow(clippy::cast_possible_truncation)]
    pub fn query() -> Result<Vec<CameraInfo>, NokhwaError> {
        Ok({
            let camera_info: Vec<CameraInfo> = v4l::context::enum_devices()
                .iter()
                .map(|node| {
                    CameraInfo::new(
                        &node
                            .name()
                            .unwrap_or(format!("{}", node.path().to_string_lossy())),
                        &format!("Video4Linux Device @ {}", node.path().to_string_lossy()),
                        "",
                        CameraIndex::Index(node.index() as u32),
                    )
                })
                .collect();
            camera_info
        })
    }

    type SharedDevice = std::sync::Arc<std::sync::Mutex<Device>>;
    type WeakSharedDevice = std::sync::Weak<std::sync::Mutex<Device>>;

    struct WeakSharedDeviceEntry {
        device: WeakSharedDevice,
        index: usize,
    }

    type SharedDeviceList = std::sync::OnceLock<std::sync::Mutex<Vec<WeakSharedDeviceEntry>>>;

    /// Global array that keep track of every Device that are currently open.
    /// This is used to open multiple handle to the same device.
    /// This is a workaround for the fact that the v4l2 backend does not support multiple handles to the same device.
    /// This replicate behavior of MF backend.
    /// The reference is a reference of Weak<Mutex<Device>>, so that the Device can be dropped when the last handle is closed.
    /// This list might need some cleanup because it will accumulate every device that is opened, this should not be a problem because the list should not grow too much.
    /// The list is also protected by a mutex, so it should be thread safe.
    static DEVICES: SharedDeviceList = std::sync::OnceLock::new();

    fn cleanup_dropped_devices(devices: &mut Vec<WeakSharedDeviceEntry>) {
        devices.retain(|entry| entry.device.strong_count() > 0);
    }

    fn new_shared_device(index: usize) -> Result<SharedDevice, NokhwaError> {
        let mut devices = DEVICES
            .get_or_init(|| std::sync::Mutex::new(Vec::new()))
            .lock()
            .map_err(|e| NokhwaError::InitializeError {
                backend: ApiBackend::Video4Linux,
                error: format!("Fail to lock global device list mutex: {}", e),
            })?;

        // do some cleanup, this will avoid here memory to grow forever
        // if for some reason someone has tons of camera plugged in
        cleanup_dropped_devices(&mut devices);

        if let Some(entry) = devices.iter().find(|entry| entry.index == index) {
            if let Some(device) = entry.device.upgrade() {
                return Ok(device);
            }
        }

        // Cleanup a second, the device we are interested might have been dropped during before upgrade call
        // For this point on we are assured that the device is not in the list
        cleanup_dropped_devices(&mut devices);

        // Let's be extra sure, this code should never panic, but maybe will help catch some race condition
        assert!(
            devices.iter().find(|entry| entry.index == index).is_none(),
            "Device {index} should not be in the list"
        );

        // Now we can open the device, and never run into a busy io error,
        // as long as the device isn't opened by other programs.
        let device = match Device::new(index) {
            Ok(dev) => dev,
            Err(why) => {
                return Err(NokhwaError::OpenDeviceError(
                    index.to_string(),
                    format!("V4L2 Error: {}", why),
                ))
            }
        };

        let device = std::sync::Arc::new(std::sync::Mutex::new(device));
        devices.push(WeakSharedDeviceEntry {
            device: std::sync::Arc::downgrade(&device),
            index,
        });

        // Last check to be sure that every devices have a unique index
        // and that the data isn't corrupted
        if devices.len() > 1 {
            let indices: std::collections::HashSet<_> = devices.iter().map(|d| d.index).collect();
            assert_eq!(
                indices.len(),
                devices.len(),
                "Device list should not contain duplicate indexes"
            );
        }

        Ok(device)
    }

    fn get_error(property: &str, why: io::Error) -> NokhwaError {
        NokhwaError::GetPropertyError {
            property: property.into(),
            error: why.to_string(),
        }
    }

    fn unsupported_parameter(error: &io::Error) -> bool {
        matches!(
            error.raw_os_error(),
            Some(libc::EINVAL) | Some(libc::ENOTTY)
        )
    }

    fn fps(interval: v4l::Fraction) -> f64 {
        if interval.numerator == 0 {
            0.0
        } else {
            f64::from(interval.denominator) / f64::from(interval.numerator)
        }
    }

    fn device_rate(device: &impl Capture) -> Result<f64, NokhwaError> {
        match device.params() {
            Ok(params) => Ok(fps(params.interval)),
            Err(e) if unsupported_parameter(&e) => Ok(0.0),
            Err(e) => Err(get_error("Frame rate", e)),
        }
    }

    fn read_mode(device: &impl Capture) -> Result<(CameraFormat, f64), NokhwaError> {
        let pix = device
            .format()
            .map_err(|e| get_error("Capture format", e))?;
        let format =
            fourcc_to_frameformat(pix.fourcc).ok_or_else(|| NokhwaError::GetPropertyError {
                property: "Frame format".into(),
                error: format!("Cannot decode {}", pix.fourcc),
            })?;
        let rate = device_rate(device)?;
        Ok((
            CameraFormat::new_from(pix.width, pix.height, format, rate.round() as u32),
            rate,
        ))
    }

    fn get_device_format(device: &impl Capture) -> Result<CameraFormat, NokhwaError> {
        read_mode(device).map(|(mode, _)| mode)
    }

    /// Reads the interval on the existing capture handle, preserving fractional rates.
    pub fn active_frame_rate(index: &CameraIndex) -> Result<f64, NokhwaError> {
        let shared = new_shared_device(index.as_index()? as usize)?;
        let device = shared
            .lock()
            .map_err(|e| NokhwaError::GeneralError(e.to_string()))?;
        device_rate(&*device)
    }

    fn advertised_formats(device: &impl Capture) -> Result<Vec<CameraFormat>, NokhwaError> {
        let mut modes = std::collections::BTreeSet::new();
        for description in device
            .enum_formats()
            .map_err(|e| get_error("Capture formats", e))?
        {
            let fourcc = description.fourcc;
            let Some(format) = fourcc_to_frameformat(fourcc) else {
                continue;
            };
            for size in device.enum_framesizes(fourcc).unwrap_or_default() {
                let mut sizes = Vec::new();
                match size.size {
                    FrameSizeEnum::Discrete(s) => sizes.push((s.width, s.height)),
                    FrameSizeEnum::Stepwise(s) => {
                        sizes.push((s.min_width, s.min_height));
                        sizes.push((s.max_width, s.max_height));
                    }
                }
                for (width, height) in sizes {
                    let intervals = device
                        .enum_frameintervals(fourcc, width, height)
                        .unwrap_or_default();
                    if intervals.is_empty() {
                        modes.insert(CameraFormat::new_from(width, height, format, 0));
                    }
                    for interval in intervals {
                        let mut add = |period| {
                            modes.insert(CameraFormat::new_from(
                                width,
                                height,
                                format,
                                fps(period).round() as u32,
                            ));
                        };
                        match interval.interval {
                            FrameIntervalEnum::Discrete(period) => add(period),
                            FrameIntervalEnum::Stepwise(range) => {
                                add(range.min);
                                add(range.max);
                            }
                        }
                    }
                }
            }
        }
        // Some devices can stream their current mode but omit it from enumeration.
        if let Ok(current) = get_device_format(device) {
            modes.insert(current);
        }
        Ok(modes.into_iter().collect())
    }

    /// Enumerate supported capture modes without programming or starting the camera.
    pub fn probe_capture_formats(index: &CameraIndex) -> Result<Vec<CameraFormat>, NokhwaError> {
        let shared = new_shared_device(index.as_index()? as usize)?;
        let device = shared
            .lock()
            .map_err(|e| NokhwaError::GeneralError(e.to_string()))?;
        let caps = device
            .query_caps()
            .map_err(|e| get_error("Device capabilities", e))?;
        // v4l 0.14 exposes device_caps. Older drivers can leave it empty.
        if !caps.capabilities.is_empty()
            && !caps
                .capabilities
                .contains(v4l::capability::Flags::VIDEO_CAPTURE)
        {
            return Ok(Vec::new());
        }
        advertised_formats(&*device)
    }

    fn negotiate_format(
        device: &impl Capture,
        requested: CameraFormat,
    ) -> Result<CameraFormat, NokhwaError> {
        let current = device
            .format()
            .map_err(|e| get_error("Capture format", e))?;
        let fourcc = if fourcc_to_frameformat(current.fourcc) == Some(requested.format()) {
            current.fourcc
        } else if requested.format() == FrameFormat::MJPEG {
            // Drivers may advertise either JPEG or MJPG for independent JPEG frames.
            device
                .enum_formats()
                .unwrap_or_default()
                .into_iter()
                .find(|f| fourcc_to_frameformat(f.fourcc) == Some(FrameFormat::MJPEG))
                .map_or(FourCC::new(b"MJPG"), |f| f.fourcc)
        } else {
            frameformat_to_fourcc(requested.format())
        };
        if (current.width, current.height, current.fourcc)
            != (requested.width(), requested.height(), fourcc)
        {
            if let Err(e) =
                device.set_format(&Format::new(requested.width(), requested.height(), fourcc))
            {
                // On EINVAL a single-mode camera can still stream its existing format.
                if e.raw_os_error() != Some(libc::EINVAL) {
                    return Err(NokhwaError::SetPropertyError {
                        property: "Capture format".into(),
                        value: requested.to_string(),
                        error: e.to_string(),
                    });
                }
            }
        }

        // S_FMT may reset the cadence. Request it using the new state and
        // allow fixed-rate cameras to keep the interval they actually grant.
        let rate = device_rate(device)?;
        if requested.frame_rate() != 0
            && rate > 0.0
            && rate.round() as u32 != requested.frame_rate()
        {
            if let Err(e) = device.set_params(&Parameters::with_fps(requested.frame_rate())) {
                if !unsupported_parameter(&e) {
                    return Err(NokhwaError::SetPropertyError {
                        property: "Frame rate".into(),
                        value: requested.frame_rate().to_string(),
                        error: e.to_string(),
                    });
                }
            }
        }
        get_device_format(device)
    }

    #[cfg(test)]
    mod tests;

    /// The backend struct that interfaces with V4L2.
    /// To see what this does, please see [`CaptureBackendTrait`].
    /// # Quirks
    /// - Calling [`set_resolution()`](CaptureBackendTrait::set_resolution), [`set_frame_rate()`](CaptureBackendTrait::set_frame_rate), or [`set_frame_format()`](CaptureBackendTrait::set_frame_format) each internally calls [`set_camera_format()`](CaptureBackendTrait::set_camera_format).
    pub struct V4LCaptureDevice<'a> {
        camera_format: CameraFormat,
        camera_info: CameraInfo,
        device: SharedDevice,
        stream_handle: Option<MmapStream<'a>>,
    }

    impl<'a> V4LCaptureDevice<'a> {
        /// Creates a new capture device using the `V4L2` backend. Indexes are gives to devices by the OS, and usually numbered by order of discovery.
        /// # Errors
        /// This function will error if the camera is currently busy or if `V4L2` can't read device information.
        #[allow(clippy::too_many_lines)]
        pub fn new(index: &CameraIndex, cam_fmt: RequestedFormat) -> Result<Self, NokhwaError> {
            let index = index.clone();

            let shared_device = new_shared_device(index.as_index()? as usize)?;
            let device = shared_device
                .lock()
                .map_err(|e| NokhwaError::InitializeError {
                    backend: ApiBackend::Video4Linux,
                    error: format!("Fail to lock device mutex: {}", e),
                })?;

            // Exact requests already contain the mode. Discovery enumerates separately.
            let modes = match cam_fmt.requested_format_type() {
                RequestedFormatType::Exact(_) => Vec::new(),
                _ => advertised_formats(&*device)?,
            };
            let requested =
                cam_fmt
                    .fulfill(&modes)
                    .ok_or_else(|| NokhwaError::GetPropertyError {
                        property: "CameraFormat".into(),
                        error: "No decodable capture mode".into(),
                    })?;
            let format = negotiate_format(&*device, requested)?;
            if format != requested {
                eprintln!("V4L2 device {index}: requested {requested}, negotiated {format}");
            }

            let device_caps = device
                .query_caps()
                .map_err(|why| NokhwaError::GetPropertyError {
                    property: "Device Capabilities".to_string(),
                    error: why.to_string(),
                })?;

            drop(device);

            Ok(V4LCaptureDevice {
                camera_format: format,
                camera_info: CameraInfo::new(
                    &device_caps.card,
                    &device_caps.driver,
                    &format!("{} {:?}", device_caps.bus, device_caps.version),
                    index,
                ),
                device: shared_device,
                stream_handle: None,
            })
        }

        /// Create a new `V4L2` Camera with desired settings. This may or may not work.
        /// # Errors
        /// This function will error if the camera is currently busy or if `V4L2` can't read device information.
        #[deprecated(since = "0.10.0", note = "please use `new` instead.")]
        #[allow(clippy::needless_pass_by_value)]
        pub fn new_with(
            index: CameraIndex,
            width: u32,
            height: u32,
            fps: u32,
            fourcc: FrameFormat,
        ) -> Result<Self, NokhwaError> {
            let camera_format = CameraFormat::new_from(width, height, fourcc, fps);
            V4LCaptureDevice::new(
                &index,
                RequestedFormat::with_formats(
                    RequestedFormatType::Exact(camera_format),
                    vec![camera_format.format()].as_slice(),
                ),
            )
        }

        fn lock_device(&self) -> Result<std::sync::MutexGuard<'_, Device>, NokhwaError> {
            self.device
                .lock()
                .map_err(|e| NokhwaError::GeneralError(format!("Failed to lock device: {}", e)))
        }

        fn get_resolution_list(&self, fourcc: FrameFormat) -> Result<Vec<Resolution>, NokhwaError> {
            let format = frameformat_to_fourcc(fourcc);

            match self.lock_device()?.enum_framesizes(format) {
                Ok(frame_sizes) => {
                    let mut resolutions = vec![];
                    for frame_size in frame_sizes {
                        match frame_size.size {
                            FrameSizeEnum::Discrete(dis) => {
                                resolutions.push(Resolution::new(dis.width, dis.height));
                            }
                            FrameSizeEnum::Stepwise(step) => {
                                resolutions.push(Resolution::new(step.min_width, step.min_height));
                                resolutions.push(Resolution::new(step.max_width, step.max_height));
                                // TODO: Respect step size
                            }
                        }
                    }
                    Ok(resolutions)
                }
                Err(why) => Err(NokhwaError::GetPropertyError {
                    property: "Resolutions".to_string(),
                    error: why.to_string(),
                }),
            }
        }

        /// Force refreshes the inner [`CameraFormat`] state.
        /// # Errors
        /// If the internal representation in the driver is invalid, this will error.
        pub fn force_refresh_camera_format(&mut self) -> Result<(), NokhwaError> {
            let camera_format = get_device_format(&*self.lock_device()?)?;
            self.camera_format = camera_format;
            Ok(())
        }
    }

    impl<'a> CaptureBackendTrait for V4LCaptureDevice<'a> {
        fn backend(&self) -> ApiBackend {
            ApiBackend::Video4Linux
        }

        fn camera_info(&self) -> &CameraInfo {
            &self.camera_info
        }

        fn refresh_camera_format(&mut self) -> Result<(), NokhwaError> {
            self.force_refresh_camera_format()
        }

        fn camera_format(&self) -> CameraFormat {
            self.camera_format
        }

        fn set_camera_format(&mut self, new_fmt: CameraFormat) -> Result<(), NokhwaError> {
            let device = self.lock_device()?;
            let prev_format = match Capture::format(&*device) {
                Ok(fmt) => fmt,
                Err(why) => {
                    return Err(NokhwaError::GetPropertyError {
                        property: "Resolution, FrameFormat".to_string(),
                        error: why.to_string(),
                    })
                }
            };
            let prev_fps = match Capture::params(&*device) {
                Ok(fps) => fps,
                Err(why) => {
                    return Err(NokhwaError::GetPropertyError {
                        property: "Frame rate".to_string(),
                        error: why.to_string(),
                    })
                }
            };

            let v4l_fcc = match new_fmt.format() {
                FrameFormat::MJPEG => FourCC::new(b"MJPG"),
                FrameFormat::YUYV => FourCC::new(b"YUYV"),
                FrameFormat::GRAY => FourCC::new(b"GRAY"),
                FrameFormat::RAWRGB => FourCC::new(b"RGB3"),
                FrameFormat::RAWBGR => FourCC::new(b"BGR3"),
                FrameFormat::NV12 => FourCC::new(b"NV12"),
            };

            let format = Format::new(new_fmt.width(), new_fmt.height(), v4l_fcc);
            let frame_rate = Parameters::with_fps(new_fmt.frame_rate());

            if let Err(why) = Capture::set_format(&*device, &format) {
                return Err(NokhwaError::SetPropertyError {
                    property: "Resolution, FrameFormat".to_string(),
                    value: format.to_string(),
                    error: why.to_string(),
                });
            }
            if let Err(why) = Capture::set_params(&*device, &frame_rate) {
                return Err(NokhwaError::SetPropertyError {
                    property: "Frame rate".to_string(),
                    value: frame_rate.to_string(),
                    error: why.to_string(),
                });
            }

            drop(device);

            if self.stream_handle.is_some() {
                return match self.open_stream() {
                    Ok(_) => Ok(()),
                    Err(why) => {
                        // undo
                        let device = self.lock_device()?;
                        if let Err(why) = Capture::set_format(&*device, &prev_format) {
                            return Err(NokhwaError::SetPropertyError {
                                property: format!("Attempt undo due to stream acquisition failure with error {}. Resolution, FrameFormat", why),
                                value: prev_format.to_string(),
                                error: why.to_string(),
                            });
                        }
                        if let Err(why) = Capture::set_params(&*device, &prev_fps) {
                            return Err(NokhwaError::SetPropertyError {
                                property:
                                format!("Attempt undo due to stream acquisition failure with error {}. Frame rate", why),
                                value: prev_fps.to_string(),
                                error: why.to_string(),
                            });
                        }
                        Err(why)
                    }
                };
            }
            self.camera_format = new_fmt;

            self.force_refresh_camera_format()?;
            if self.camera_format != new_fmt {
                return Err(NokhwaError::SetPropertyError {
                    property: "CameraFormat".to_string(),
                    value: new_fmt.to_string(),
                    error: "Rejected".to_string(),
                });
            }

            Ok(())
        }

        fn compatible_list_by_resolution(
            &mut self,
            fourcc: FrameFormat,
        ) -> Result<HashMap<Resolution, Vec<u32>>, NokhwaError> {
            let resolutions = self.get_resolution_list(fourcc)?;
            let format = frameformat_to_fourcc(fourcc);
            let mut res_map = HashMap::new();
            for res in resolutions {
                let mut compatible_fps = vec![];
                match self
                    .lock_device()?
                    .enum_frameintervals(format, res.width(), res.height())
                {
                    Ok(intervals) => {
                        for interval in intervals {
                            match interval.interval {
                                FrameIntervalEnum::Discrete(dis) => {
                                    compatible_fps.push(dis.denominator);
                                }
                                FrameIntervalEnum::Stepwise(step) => {
                                    for fstep in (step.min.numerator..step.max.numerator)
                                        .step_by(step.step.numerator as usize)
                                    {
                                        if step.max.denominator != 1 || step.min.denominator != 1 {
                                            compatible_fps.push(fstep);
                                        }
                                    }
                                }
                            }
                        }
                    }
                    Err(why) => {
                        return Err(NokhwaError::GetPropertyError {
                            property: "Frame rate".to_string(),
                            error: why.to_string(),
                        })
                    }
                }
                res_map.insert(res, compatible_fps);
            }
            Ok(res_map)
        }

        fn compatible_fourcc(&mut self) -> Result<Vec<FrameFormat>, NokhwaError> {
            match self.lock_device()?.enum_formats() {
                Ok(formats) => {
                    let mut frame_format_vec = vec![];
                    for format in formats {
                        match fourcc_to_frameformat(format.fourcc) {
                            Some(ff) => frame_format_vec.push(ff),
                            None => continue,
                        }
                    }
                    frame_format_vec.sort();
                    frame_format_vec.dedup();
                    Ok(frame_format_vec)
                }
                Err(why) => Err(NokhwaError::GetPropertyError {
                    property: "FrameFormat".to_string(),
                    error: why.to_string(),
                }),
            }
        }

        fn resolution(&self) -> Resolution {
            self.camera_format.resolution()
        }

        fn set_resolution(&mut self, new_res: Resolution) -> Result<(), NokhwaError> {
            let mut new_fmt = self.camera_format;
            new_fmt.set_resolution(new_res);
            self.set_camera_format(new_fmt)
        }

        fn frame_rate(&self) -> u32 {
            self.camera_format.frame_rate()
        }

        fn set_frame_rate(&mut self, new_fps: u32) -> Result<(), NokhwaError> {
            let mut new_fmt = self.camera_format;
            new_fmt.set_frame_rate(new_fps);
            self.set_camera_format(new_fmt)
        }

        fn frame_format(&self) -> FrameFormat {
            self.camera_format.format()
        }

        fn set_frame_format(&mut self, fourcc: FrameFormat) -> Result<(), NokhwaError> {
            let mut new_fmt = self.camera_format;
            new_fmt.set_format(fourcc);
            self.set_camera_format(new_fmt)
        }

        fn camera_control(
            &self,
            control: KnownCameraControl,
        ) -> Result<CameraControl, NokhwaError> {
            let controls = self.camera_controls()?;
            for supported_control in controls {
                if supported_control.control() == control {
                    return Ok(supported_control);
                }
            }
            Err(NokhwaError::GetPropertyError {
                property: control.to_string(),
                error: "not found/not supported".to_string(),
            })
        }

        #[allow(clippy::cast_possible_wrap)]
        fn camera_controls(&self) -> Result<Vec<CameraControl>, NokhwaError> {
            let device = self.lock_device()?;
            device
                .query_controls()
                .map_err(|why| NokhwaError::GetPropertyError {
                    property: "V4L2 Controls".to_string(),
                    error: why.to_string(),
                })?
                .into_iter()
                .map(|desc| {
                    let id_as_kcc = id_to_known_camera_control(desc.id);
                    let ctrl_current = device.control(desc.id)?.value;

                    let ctrl_value_desc = match (desc.typ, ctrl_current) {
                        (
                            Type::Integer
                            | Type::Integer64
                            | Type::Menu
                            | Type::U8
                            | Type::U16
                            | Type::U32
                            | Type::IntegerMenu,
                            Value::Integer(current),
                        ) => ControlValueDescription::IntegerRange {
                            min: desc.minimum as i64,
                            max: desc.maximum,
                            value: current,
                            step: desc.step as i64,
                            default: desc.default,
                        },
                        (Type::Boolean, Value::Boolean(current)) => {
                            ControlValueDescription::Boolean {
                                value: current,
                                default: desc.default != 0,
                            }
                        }

                        (Type::String, Value::String(current)) => ControlValueDescription::String {
                            value: current,
                            default: None,
                        },
                        _ => {
                            return Err(io::Error::new(
                                ErrorKind::Unsupported,
                                "what is this?????? todo: support ig",
                            ))
                        }
                    };

                    let is_readonly = desc
                        .flags
                        .intersects(Flags::READ_ONLY)
                        .then_some(KnownCameraControlFlag::ReadOnly);
                    let is_writeonly = desc
                        .flags
                        .intersects(Flags::WRITE_ONLY)
                        .then_some(KnownCameraControlFlag::WriteOnly);
                    let is_disabled = desc
                        .flags
                        .intersects(Flags::DISABLED)
                        .then_some(KnownCameraControlFlag::Disabled);
                    let is_volatile = desc
                        .flags
                        .intersects(Flags::VOLATILE)
                        .then_some(KnownCameraControlFlag::Volatile);
                    let is_inactive = desc
                        .flags
                        .intersects(Flags::INACTIVE)
                        .then_some(KnownCameraControlFlag::Disabled);
                    let flags_vec = vec![
                        is_inactive,
                        is_readonly,
                        is_volatile,
                        is_disabled,
                        is_writeonly,
                    ]
                    .into_iter()
                    .filter(Option::is_some)
                    .collect::<Option<Vec<KnownCameraControlFlag>>>()
                    .unwrap_or_default();

                    Ok(CameraControl::new(
                        id_as_kcc,
                        desc.name,
                        ctrl_value_desc,
                        flags_vec,
                        !desc.flags.intersects(Flags::INACTIVE),
                    ))
                })
                .filter(Result::is_ok)
                .collect::<Result<Vec<CameraControl>, io::Error>>()
                .map_err(|x| NokhwaError::GetPropertyError {
                    property: "www".to_string(),
                    error: x.to_string(),
                })
        }

        fn set_camera_control(
            &mut self,
            id: KnownCameraControl,
            value: ControlValueSetter,
        ) -> Result<(), NokhwaError> {
            let conv_value = match value.clone() {
                ControlValueSetter::None => Value::None,
                ControlValueSetter::Integer(i) => Value::Integer(i),
                ControlValueSetter::Boolean(b) => Value::Boolean(b),
                ControlValueSetter::String(s) => Value::String(s),
                ControlValueSetter::Bytes(b) => Value::CompoundU8(b),
                v => {
                    return Err(NokhwaError::SetPropertyError {
                        property: id.to_string(),
                        value: v.to_string(),
                        error: "not supported".to_string(),
                    })
                }
            };
            self.lock_device()?
                .set_control(Control {
                    id: known_camera_control_to_id(id),
                    value: conv_value,
                })
                .map_err(|why| NokhwaError::SetPropertyError {
                    property: id.to_string(),
                    value: format!("{:?}", value),
                    error: why.to_string(),
                })?;
            // verify

            let control = self.camera_control(id)?;
            if control.value() != value {
                return Err(NokhwaError::SetPropertyError {
                    property: id.to_string(),
                    value: format!("{:?}", value),
                    error: "Rejected".to_string(),
                });
            }
            Ok(())
        }

        fn open_stream(&mut self) -> Result<(), NokhwaError> {
            // Disable mut warning, since mut is only required when not using arena buffers
            #[allow(unused_mut)]
            let mut stream =
                match MmapStream::new(&*self.lock_device()?, v4l::buffer::Type::VideoCapture) {
                    Ok(s) => s,
                    Err(why) => return Err(NokhwaError::OpenStreamError(why.to_string())),
                };

            // Explicitly start now, or won't work with the RPi. As a consequence, buffers will only be used as required.
            // WARNING: This will cause drop of half of the frames
            #[cfg(feature = "no-arena-buffer")]
            match stream.start() {
                Ok(s) => s,
                Err(why) => return Err(NokhwaError::OpenStreamError(why.to_string())),
            }
            self.stream_handle = Some(stream);
            Ok(())
        }

        fn is_stream_open(&self) -> bool {
            self.stream_handle.is_some()
        }

        fn frame(&mut self) -> Result<Buffer, NokhwaError> {
            let cam_fmt = self.camera_format;
            match &mut self.stream_handle {
                Some(sh) => match sh.next() {
                    Ok((data, meta)) => {
                        let wall_ts = monotonic_to_wallclock(meta.timestamp);
                        Ok(Buffer::with_timestamp(
                            cam_fmt.resolution(),
                            data,
                            cam_fmt.format(),
                            wall_ts,
                        ))
                    }
                    Err(why) => Err(NokhwaError::ReadFrameError(why.to_string())),
                },
                None => Err(NokhwaError::ReadFrameError(
                    "Stream Not Started".to_string(),
                )),
            }
        }

        fn frame_raw(&mut self) -> Result<Cow<'_, [u8]>, NokhwaError> {
            match &mut self.stream_handle {
                Some(sh) => match sh.next() {
                    Ok((data, _)) => Ok(Cow::Borrowed(data)),
                    Err(why) => Err(NokhwaError::ReadFrameError(why.to_string())),
                },
                None => Err(NokhwaError::ReadFrameError(
                    "Stream Not Started".to_string(),
                )),
            }
        }

        fn stop_stream(&mut self) -> Result<(), NokhwaError> {
            if self.stream_handle.is_some() {
                self.stream_handle = None;
            }
            Ok(())
        }
    }

    fn fourcc_to_frameformat(fourcc: FourCC) -> Option<FrameFormat> {
        match fourcc.str().ok()? {
            "YUYV" => Some(FrameFormat::YUYV),
            "MJPG" | "JPEG" => Some(FrameFormat::MJPEG),
            "GRAY" => Some(FrameFormat::GRAY),
            "RGB3" => Some(FrameFormat::RAWRGB),
            "BGR3" => Some(FrameFormat::RAWBGR),
            "NV12" => Some(FrameFormat::NV12),
            _ => None,
        }
    }

    fn frameformat_to_fourcc(fourcc: FrameFormat) -> FourCC {
        match fourcc {
            FrameFormat::MJPEG => FourCC::new(b"MJPG"),
            FrameFormat::YUYV => FourCC::new(b"YUYV"),
            FrameFormat::GRAY => FourCC::new(b"GRAY"),
            FrameFormat::RAWRGB => FourCC::new(b"RGB3"),
            FrameFormat::RAWBGR => FourCC::new(b"BGR3"),
            FrameFormat::NV12 => FourCC::new(b"NV12"),
        }
    }

    /// Convert a V4L2 CLOCK_MONOTONIC timestamp to a wallclock Duration since UNIX_EPOCH.
    fn monotonic_to_wallclock(ts: v4l::Timestamp) -> Option<std::time::Duration> {
        let frame_mono = std::time::Duration::from(ts);
        if frame_mono.is_zero() {
            return None;
        }

        let mut mono_now = libc::timespec {
            tv_sec: 0,
            tv_nsec: 0,
        };
        let mut wall_now = libc::timespec {
            tv_sec: 0,
            tv_nsec: 0,
        };
        // SAFETY: passing valid pointers to kernel clock_gettime
        unsafe {
            libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut mono_now);
            libc::clock_gettime(libc::CLOCK_REALTIME, &mut wall_now);
        }
        let mono_now = std::time::Duration::new(mono_now.tv_sec as u64, mono_now.tv_nsec as u32);
        let wall_now = std::time::Duration::new(wall_now.tv_sec as u64, wall_now.tv_nsec as u32);

        // frame_age = how long ago the frame was captured (monotonic delta)
        let frame_age = mono_now.checked_sub(frame_mono)?;
        wall_now.checked_sub(frame_age)
    }
}

#[cfg(not(target_os = "linux"))]
mod internal {
    use nokhwa_core::buffer::Buffer;
    use nokhwa_core::error::NokhwaError;
    use nokhwa_core::traits::CaptureBackendTrait;
    use nokhwa_core::types::{
        ApiBackend, CameraControl, CameraFormat, CameraIndex, CameraInfo, ControlValueSetter,
        FrameFormat, KnownCameraControl, RequestedFormat, Resolution,
    };
    use std::borrow::Cow;
    use std::collections::HashMap;
    use std::marker::PhantomData;

    /// Attempts to convert a [`KnownCameraControl`] into a V4L2 Control ID.
    /// If the associated control is not found, this will return `None` (`ColorEnable`, `Roll`)
    #[allow(clippy::cast_possible_truncation)]
    pub fn known_camera_control_to_id(_ctrl: KnownCameraControl) -> u32 {
        0
    }

    /// Attempts to convert a [`u32`] V4L2 Control ID into a [`KnownCameraControl`]
    /// If the associated control is not found, this will return `None` (`ColorEnable`, `Roll`)
    #[allow(clippy::cast_lossless)]
    pub fn id_to_known_camera_control(id: u32) -> KnownCameraControl {
        KnownCameraControl::Other(id as u128)
    }

    /// The backend struct that interfaces with V4L2.
    /// To see what this does, please see [`CaptureBackendTrait`].
    /// # Quirks
    /// - Calling [`set_resolution()`](CaptureBackendTrait::set_resolution), [`set_frame_rate()`](CaptureBackendTrait::set_frame_rate), or [`set_frame_format()`](CaptureBackendTrait::set_frame_format) each internally calls [`set_camera_format()`](CaptureBackendTrait::set_camera_format).
    pub struct V4LCaptureDevice<'a> {
        __holder: PhantomData<&'a str>,
    }

    #[allow(unused_variables)]
    impl<'a> V4LCaptureDevice<'a> {
        /// Creates a new capture device using the `V4L2` backend. Indexes are gives to devices by the OS, and usually numbered by order of discovery.
        /// # Errors
        /// This function will error if the camera is currently busy or if `V4L2` can't read device information.
        #[allow(clippy::too_many_lines)]
        pub fn new(index: &CameraIndex, cam_fmt: RequestedFormat) -> Result<Self, NokhwaError> {
            Err(NokhwaError::NotImplementedError(
                "V4L2 only on Linux".to_string(),
            ))
        }

        /// Create a new `V4L2` Camera with desired settings. This may or may not work.
        /// # Errors
        /// This function will error if the camera is currently busy or if `V4L2` can't read device information.
        #[deprecated(since = "0.10.0", note = "please use `new` instead.")]
        pub fn new_with(
            index: CameraIndex,
            width: u32,
            height: u32,
            fps: u32,
            fourcc: FrameFormat,
        ) -> Result<Self, NokhwaError> {
            Err(NokhwaError::NotImplementedError(
                "V4L2 only on Linux".to_string(),
            ))
        }

        /// Force refreshes the inner [`CameraFormat`] state.
        /// # Errors
        /// If the internal representation in the driver is invalid, this will error.
        pub fn force_refresh_camera_format(&mut self) -> Result<(), NokhwaError> {
            Err(NokhwaError::NotImplementedError(
                "V4L2 only on Linux".to_string(),
            ))
        }
    }

    #[allow(unused_variables)]
    impl<'a> CaptureBackendTrait for V4LCaptureDevice<'a> {
        fn backend(&self) -> ApiBackend {
            ApiBackend::Video4Linux
        }

        fn camera_info(&self) -> &CameraInfo {
            todo!()
        }

        fn refresh_camera_format(&mut self) -> Result<(), NokhwaError> {
            todo!()
        }

        fn camera_format(&self) -> CameraFormat {
            todo!()
        }

        fn set_camera_format(&mut self, new_fmt: CameraFormat) -> Result<(), NokhwaError> {
            todo!()
        }

        fn compatible_list_by_resolution(
            &mut self,
            fourcc: FrameFormat,
        ) -> Result<HashMap<Resolution, Vec<u32>>, NokhwaError> {
            todo!()
        }

        fn compatible_fourcc(&mut self) -> Result<Vec<FrameFormat>, NokhwaError> {
            todo!()
        }

        fn resolution(&self) -> Resolution {
            todo!()
        }

        fn set_resolution(&mut self, new_res: Resolution) -> Result<(), NokhwaError> {
            todo!()
        }

        fn frame_rate(&self) -> u32 {
            todo!()
        }

        fn set_frame_rate(&mut self, new_fps: u32) -> Result<(), NokhwaError> {
            todo!()
        }

        fn frame_format(&self) -> FrameFormat {
            todo!()
        }

        fn set_frame_format(&mut self, fourcc: FrameFormat) -> Result<(), NokhwaError> {
            todo!()
        }

        fn camera_control(
            &self,
            control: KnownCameraControl,
        ) -> Result<CameraControl, NokhwaError> {
            todo!()
        }

        fn camera_controls(&self) -> Result<Vec<CameraControl>, NokhwaError> {
            todo!()
        }

        fn set_camera_control(
            &mut self,
            id: KnownCameraControl,
            value: ControlValueSetter,
        ) -> Result<(), NokhwaError> {
            todo!()
        }

        fn open_stream(&mut self) -> Result<(), NokhwaError> {
            todo!()
        }

        fn is_stream_open(&self) -> bool {
            todo!()
        }

        fn frame(&mut self) -> Result<Buffer, NokhwaError> {
            todo!()
        }

        fn frame_raw(&mut self) -> Result<Cow<'_, [u8]>, NokhwaError> {
            todo!()
        }

        fn stop_stream(&mut self) -> Result<(), NokhwaError> {
            todo!()
        }
    }
}

pub use internal::*;
