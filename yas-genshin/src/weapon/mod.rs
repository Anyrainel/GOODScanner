pub use weapon::GenshinWeapon;
pub use weapon::{parse_level_and_ascension, parse_refinement};
pub use zh_cn::weapon_name_to_good;

pub mod weapon;
mod zh_cn;
