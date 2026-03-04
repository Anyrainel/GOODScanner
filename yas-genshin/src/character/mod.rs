pub use character_names::{CHARACTER_NAMES, CHARACTER_ALIASES};
pub use character::GenshinCharacter;
pub use name_mapping::{zh_cn_to_good_key, element_from_zh_cn, element_from_color};

mod character_names;
pub mod character;
pub mod name_mapping;
