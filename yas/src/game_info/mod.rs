mod game_info;
mod game_info_builder;
mod os;
mod resolution_family;
mod ui;

pub use game_info::GameInfo;
pub use game_info_builder::GameInfoBuilder;
pub use resolution_family::is_16x9;
pub use ui::{Platform, UI};
