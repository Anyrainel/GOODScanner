use std::time::Duration;

use anyhow::Result;

pub trait ImageToText<ImageType>: Send + Sync {
    fn image_to_text(&self, image: &ImageType, is_preprocessed: bool) -> Result<String>;

    fn get_average_inference_time(&self) -> Option<Duration>;

    /// Identity of the underlying weights, so callers can skip re-running an
    /// identical model on an identical crop. `None` means unknown, which
    /// callers must treat as distinct from every other model.
    fn model_id(&self) -> Option<&str> {
        None
    }
}

// pub trait ImageTextDetection<ImageType> {
//
// }
