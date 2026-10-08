use clap::Parser;
use good_tools_app::{
    config::{Game, ToolTab},
    gui::{preview, state::Lang},
};

#[derive(Parser)]
struct Args {
    #[arg(long)]
    output: std::path::PathBuf,
    #[arg(long, default_value = "genshin")]
    game: String,
    #[arg(long, default_value = "scanner")]
    tab: String,
    #[arg(long, default_value = "zh")]
    lang: String,
    #[arg(long, default_value_t = 1040.0)]
    width: f32,
    #[arg(long, default_value_t = 760.0)]
    height: f32,
    #[arg(long)]
    completed: bool,
    #[arg(long, default_value = "ready")]
    scenario: String,
    /// Click a control in the real UI before taking the screenshot.
    #[arg(long, num_args = 2)]
    click: Vec<f32>,
}

fn main() -> eframe::Result {
    let args = Args::parse();
    let game = match args.game.as_str() {
        "genshin" => Game::Genshin,
        "star-rail" => Game::StarRail,
        _ => panic!("unsupported game"),
    };
    let tab = match args.tab.as_str() {
        "scanner" => ToolTab::Scanner,
        "manager" => ToolTab::Manager,
        "capture" => ToolTab::Capture,
        "about" => ToolTab::Credits,
        _ => panic!("unsupported tab"),
    };
    let lang = match args.lang.as_str() {
        "zh" => Lang::Zh,
        "en" => Lang::En,
        _ => panic!("unsupported language"),
    };
    preview::run(
        args.output,
        game,
        tab,
        lang,
        [args.width, args.height],
        args.completed,
        (!args.click.is_empty()).then(|| [args.click[0], args.click[1]]),
        args.scenario,
    )
}
