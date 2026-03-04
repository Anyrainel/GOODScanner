use image::RgbImage;

pub struct WeaponSendItem {
    pub panel_image: RgbImage,
    pub star: usize,
    pub list_image: Option<RgbImage>,
}
