use std::path::PathBuf;

use alloy::signers::local::PrivateKeySigner;
use clap::{ArgAction, Parser};
use color_eyre::eyre::{bail, Context, Result};
use tracing::{info, instrument, warn};
use tracing_subscriber::EnvFilter;

mod commands;
mod config;

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
pub(crate) struct Args {
    /// Path to config file
    #[arg(short, long, global = true, display_order(0))]
    pub config: Option<PathBuf>,
    /// Ethereum private keys to use
    #[arg(long, global = true, action(ArgAction::Append), display_order(0))]
    pub pk: Option<Vec<String>>,
    /// Increases the level of verbosity (the max level is -vvv).
    #[arg(short, global = true, action = ArgAction::Count, display_order(99))]
    pub verbose: u8,

    #[command(subcommand)]
    pub cmd: commands::Command,
}

impl Args {
    /// Build list of signers from the cli arguments
    fn build_signers(&self) -> Result<Vec<PrivateKeySigner>> {
        let Some(accounts) = &self.pk else {
            return Ok(vec![]);
        };
        if accounts.len() < 2 {
            bail!("At least 2 ethereum keys are required");
        }
        accounts
            .iter()
            .map(|s| {
                s.parse::<PrivateKeySigner>()
                    .inspect(|v| {
                        info!("Using Ethereum Account: {}", v.address());
                    })
                    .with_context(|| format!("failed to parse key: {s}"))
            })
            .collect()
    }
}

#[tokio::main]
#[instrument]
async fn main() -> Result<()> {
    color_eyre::install()?;

    // Parse cli arguments and app setup
    let args = Args::parse();

    // Setup logging filters
    let env_filter = EnvFilter::builder().parse_lossy(match std::env::var("RUST_LOG") {
        // Environment override
        Ok(filter) => filter,
        // Default which is directed by the verbosity flag
        Err(_) => match args.verbose {
            0 => "info",
            1 => "debug",
            _ => "trace",
        }
        .to_string(),
    });
    let _ = tracing_subscriber::fmt()
        .with_env_filter(env_filter)
        .with_target(true)
        .with_thread_ids(false)
        .with_file(false)
        .with_line_number(false)
        .compact()
        .try_init();

    // Load config, build eth signers, and run the given command
    let config = config::Config::load(args.config.as_ref())?;
    let signers = args.build_signers()?;
    args.cmd.execute(config, signers).await
}
