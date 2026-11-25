use super::*;
use std::cell::Cell;
use v4l::{
    format::Description,
    frameinterval::FrameInterval,
    framesize::{Discrete, FrameSize},
    Fraction,
};

struct FixedCamera {
    fourcc: FourCC,
    interval: Fraction,
    set_error: Option<i32>,
    format_writes: Cell<u32>,
    rate_writes: Cell<u32>,
}

impl FixedCamera {
    fn new(fourcc: &[u8; 4], interval: Fraction) -> Self {
        Self {
            fourcc: FourCC::new(fourcc),
            interval,
            set_error: None,
            format_writes: Cell::new(0),
            rate_writes: Cell::new(0),
        }
    }
}

impl Capture for FixedCamera {
    fn enum_formats(&self) -> io::Result<Vec<Description>> {
        Ok(vec![Description {
            index: 0,
            typ: 1,
            flags: v4l::format::description::Flags::COMPRESSED,
            description: "JPEG".into(),
            fourcc: self.fourcc,
        }])
    }

    fn enum_framesizes(&self, fourcc: FourCC) -> io::Result<Vec<FrameSize>> {
        Ok([(1920, 1080), (1280, 720), (1920, 1080)]
            .into_iter()
            .enumerate()
            .map(|(index, (width, height))| FrameSize {
                index: index as u32,
                fourcc,
                typ: 1,
                size: FrameSizeEnum::Discrete(Discrete { width, height }),
            })
            .collect())
    }

    fn enum_frameintervals(
        &self,
        fourcc: FourCC,
        width: u32,
        height: u32,
    ) -> io::Result<Vec<FrameInterval>> {
        Ok(vec![FrameInterval {
            index: 0,
            fourcc,
            width,
            height,
            typ: 1,
            interval: FrameIntervalEnum::Discrete(Fraction::new(1, 30)),
        }])
    }

    fn format(&self) -> io::Result<Format> {
        Ok(Format::new(1280, 720, self.fourcc))
    }

    fn set_format(&self, format: &Format) -> io::Result<Format> {
        self.format_writes.set(self.format_writes.get() + 1);
        assert_eq!(format.fourcc, self.fourcc);
        if let Some(code) = self.set_error {
            return Err(io::Error::from_raw_os_error(code));
        }
        self.format()
    }

    fn params(&self) -> io::Result<Parameters> {
        Ok(Parameters::new(self.interval))
    }

    fn set_params(&self, _: &Parameters) -> io::Result<Parameters> {
        self.rate_writes.set(self.rate_writes.get() + 1);
        Err(io::Error::from_raw_os_error(libc::EINVAL))
    }
}

fn requested() -> CameraFormat {
    CameraFormat::new_from(1920, 1080, FrameFormat::MJPEG, 30)
}

#[test]
fn probe_is_read_only_and_deduplicates_modes() {
    let camera = FixedCamera::new(b"MJPG", Fraction::new(1, 30));
    let modes = advertised_formats(&camera).unwrap();
    assert_eq!(modes.len(), 2);
    assert!(modes.contains(&requested()));
    assert!(modes.contains(&CameraFormat::new_from(1280, 720, FrameFormat::MJPEG, 30)));
    assert_eq!(
        (camera.format_writes.get(), camera.rate_writes.get()),
        (0, 0)
    );
}

#[test]
fn uses_the_granted_resolution_and_fixed_frame_rate() {
    let camera = FixedCamera::new(b"JPEG", Fraction::new(1, 15));
    assert_eq!(
        negotiate_format(&camera, requested()).unwrap(),
        CameraFormat::new_from(1280, 720, FrameFormat::MJPEG, 15)
    );
    assert_eq!(
        (camera.format_writes.get(), camera.rate_writes.get()),
        (1, 1)
    );
}

#[test]
fn a_rejected_change_can_use_the_existing_mode_with_fractional_fps() {
    let mut camera = FixedCamera::new(b"MJPG", Fraction::new(1001, 30000));
    camera.set_error = Some(libc::EINVAL);
    let granted = negotiate_format(&camera, requested()).unwrap();
    assert_eq!(granted.width(), 1280);
    assert_eq!(read_mode(&camera).unwrap().1, 30000.0 / 1001.0);
    assert_eq!(camera.rate_writes.get(), 0);
}

#[test]
fn a_busy_device_still_returns_an_error() {
    let mut camera = FixedCamera::new(b"MJPG", Fraction::new(1, 30));
    camera.set_error = Some(libc::EBUSY);
    assert!(negotiate_format(&camera, requested()).is_err());
}
