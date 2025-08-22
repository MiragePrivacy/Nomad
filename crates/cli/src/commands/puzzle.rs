use std::io::{stdout, Write};
use std::path::PathBuf;

use alloy::hex;
use alloy::signers::k256::sha2::{Digest, Sha256};
use alloy::signers::local::PrivateKeySigner;
use clap::{Args, ValueEnum};
use color_eyre::eyre::bail;
use color_eyre::Result;
use rand::rngs::StdRng;
use rand::{RngCore, SeedableRng};
use tracing::info;

use nomad_puzzle::PuzzleGenerator;

use crate::config::Config;

#[derive(ValueEnum, Clone, Default, PartialEq, Eq)]
pub enum OutputFormat {
    /// Output raw program bytecode
    #[default]
    Bytes,
    /// Output commented mnemonic text to stdout
    Mnemonic,
    /// Output mermaid graph text to stdout
    Mermaid,
}

#[derive(Args)]
pub struct PuzzleArgs {
    /// Seed for puzzle generation. If not provided, uses a random seed.
    #[arg(short, long)]
    pub seed: Option<u64>,

    /// Maximum recursion depth for transformations
    #[arg(short, long, default_value = "8")]
    pub depth: usize,

    /// Maximum number of instructions
    #[arg(long("max"), default_value_t = 1024 * 1024)]
    pub max_instructions: usize,

    /// Path to write puzzle output to
    #[arg(short, long)]
    pub out: Option<PathBuf>,

    /// Enable a debug mode outputting a mermaid graph or commented mnemonics
    #[arg(short, long, value_enum, default_value_t = Default::default())]
    pub format: OutputFormat,

    /// Target 256-bit output for the puzzle (32 hex bytes).
    #[arg(value_parser = parse_hex_bytes)]
    pub target: [u8; 32],
}

impl PuzzleArgs {
    pub async fn execute(self, _config: Config, _signers: Vec<PrivateKeySigner>) -> Result<()> {
        if self.format == OutputFormat::Bytes && self.out.is_none() {
            bail!("Bytecode output requires -o/--output <PATH>");
        }

        // Use provided seed or generate a random one
        let seed = self.seed.unwrap_or_else(|| rand::rng().next_u64());

        // Hash 64 bit seed into 256 bits
        let seed_bytes = Sha256::digest(seed.to_be_bytes());
        let rng = StdRng::from_seed(seed_bytes.into());

        info!(
            seed,
            depth = self.depth,
            max_instructions = self.max_instructions,
            "Generating puzzle"
        );

        let mut generator = PuzzleGenerator::new(self.depth, self.max_instructions, rng);
        let writer = &mut writer(self.out)?;
        match self.format {
            OutputFormat::Bytes => {
                generator.generate(self.target)?.encode(writer)?;
            }
            OutputFormat::Mnemonic => {
                writer.write_all(generator.generate_mnemonic(self.target)?.as_bytes())?;
            }
            OutputFormat::Mermaid => {
                writer.write_all(generator.generate_mermaid(self.target)?.as_bytes())?;
            }
        }

        Ok(())
    }
}

/// Open a writer to a file or stdout if not provided
fn writer(out: Option<PathBuf>) -> Result<Box<dyn Write>> {
    match out {
        Some(path) => {
            let file = std::fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .open(path)?;
            Ok(Box::new(file))
        }
        None => Ok(Box::new(stdout())),
    }
}

/// Parse hex string into 32-byte array
fn parse_hex_bytes(s: &str) -> Result<[u8; 32]> {
    let s = s.strip_prefix("0x").unwrap_or(s);
    if s.len() != 64 {
        bail!("Expected 64 hex characters (32 bytes), got {}", s.len());
    }

    let bytes = hex::decode(s)?;
    if bytes.len() != 32 {
        bail!("Expected 32 bytes, got {}", bytes.len());
    }

    let mut result = [0u8; 32];
    result.copy_from_slice(&bytes);
    Ok(result)
}

