use std::{
    fs,
    io::{BufReader, Read},
    path::PathBuf,
};

use anyhow::{Result, anyhow};
use auto_artifactarium::{
    matches_avatars_all_data_notify, matches_item_packet, matches_items_all_data_notify,
};
use clap::{Args, Parser, Subcommand, ValueEnum};

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Avatars { path: PathBuf },
    Items { path: PathBuf },
}

fn read_file(path: PathBuf) -> Result<Vec<u8>> {
    let f = fs::File::open(path)?;
    let mut reader = BufReader::new(f);
    let mut buffer = Vec::new();
    reader.read_to_end(&mut buffer)?;
    Ok(buffer)
}

fn avatars_command(path: PathBuf) -> Result<()> {
    let buffer = read_file(path)?;

    let avatars = matches_avatars_all_data_notify(&buffer)
        .ok_or_else(|| anyhow!("unable to parse data as avatars"))?;
    println!("{avatars:#?}");

    Ok(())
}

fn items_command(path: PathBuf) -> Result<()> {
    let buffer = read_file(path)?;

    let items = matches_items_all_data_notify(&buffer)
        .ok_or_else(|| anyhow!("unable to parse data as items"))?;
    println!("{items:#?}");

    Ok(())
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::Avatars { path } => avatars_command(path),
        Command::Items { path } => items_command(path),
    }
}
