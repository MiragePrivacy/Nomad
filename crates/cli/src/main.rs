use std::path::PathBuf;

use alloy::signers::local::PrivateKeySigner;
use clap::{ArgAction, Parser};
use color_eyre::eyre::{bail, Context, Result};
use tracing::{info, trace};
use tracing_subscriber::EnvFilter;
use workspace_filter::workspace_filter;

mod commands;
mod config;

#[derive(Parser)]
#[command(author, version, about)]
pub(crate) struct Args {
    /// Path to config file
    #[arg(short, long, global = true, display_order(0))]
    pub config: Option<PathBuf>,
    /// Ethereum private keys to use
    #[arg(long, global = true, action(ArgAction::Append), display_order(0))]
    pub pk: Option<Vec<String>>,
    /// Increases the level of verbosity. Max value is -vvvv.
    ///
    /// * Default: All crates at info level
    /// * -v     : Nomad crates at debug level, all others at info
    /// * -vv    : Nomad crates at trace level, all others at info
    /// * -vvv   : Nomad crates at trace level, all others at debug
    /// * -vvvv  : All crates at trace level
    #[arg(short, global = true, action = ArgAction::Count, display_order(99))]
    #[clap(verbatim_doc_comment)]
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

    // Setup logging filters and subscriber
    pub fn setup_logging(&self) {
        let filter = std::env::var("RUST_LOG").unwrap_or_else(|_| {
            // Default which is directed by the verbosity flag
            match self.verbose {
                0 => "info".into(),
                1 => workspace_filter!("debug", "info,nomad={level}"),
                2 => workspace_filter!("trace", "info,nomad={level}"),
                3 => workspace_filter!("trace", "debug,nomad={level}"),
                _ => "trace".into(),
            }
        });
        let filter = EnvFilter::builder().parse_lossy(filter);
        let env_filter = filter.to_string();

        // Init subscriber
        let _ = tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_target(self.verbose > 1)
            .with_thread_ids(false)
            .with_file(false)
            .with_line_number(false)
            .compact()
            .try_init();

        trace!(env_filter);
    }

    // Setup and execute the given command
    pub async fn execute(self) -> Result<()> {
        color_eyre::install()?;
        self.setup_logging();
        let config = config::Config::load(self.config.as_ref())?;
        let signers = self.build_signers()?;
        self.cmd.execute(config, signers).await
    }
}

fn main() -> Result<()> {
    tokio::runtime::Runtime::new()?.block_on(Args::parse().execute())
}
