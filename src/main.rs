use std::{fs, path::PathBuf};

use clap::Parser;
use serde::Deserialize;

#[derive(Parser)]
#[command(version, about)]
struct Cli {
    /// Config TOML file
    pub config: PathBuf,
}

/// Config TOML
#[derive(Deserialize)]
pub struct Config {
    addresses: Addresses,
}

/// Secret Addresses
#[derive(Deserialize)]
struct Addresses {
    execution_pk: String,
    validation_pk: String,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    let raw_config = fs::read_to_string(cli.config)?;

    let config: Config = toml::from_str(&raw_config)?;

    println!("{}", config.addresses.execution_pk);

    Ok(())
}
