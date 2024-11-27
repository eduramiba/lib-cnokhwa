use nokhwa::utils::{FrameFormat};

#[derive(Debug, PartialEq, Eq, Clone, Hash, Ord, PartialOrd)]
pub struct VideoFormat {
    pub index: usize,
    pub width: u32,
    pub height: u32,
    pub format: FrameFormat,
    pub frame_rate: u32
}