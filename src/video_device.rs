use nokhwa::utils::CameraIndex;
use crate::video_format::VideoFormat;

#[derive(Debug, PartialEq, Eq, Clone, Hash)]
pub struct VideoDevice {
    pub index: CameraIndex,
    pub unique_id: String,
    pub model_id: String,
    pub name: String,
    pub formats: Vec<VideoFormat>
}