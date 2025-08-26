use alloy::primitives::{Address, U256};
use clap::Parser;
use color_eyre::eyre::{Context, Result};
use reqwest::Url;
use tracing::info;

use nomad_rpc::{HttpClient, MirageRpcClient};
use nomad_types::{SignalPayload, Signal};

#[derive(Parser)]
pub struct SignalArgs {
    /// Escrow contract address
    escrow: Address,
    /// Token contract address
    token: Address,
    /// Transfer recipient address
    recipient: Address,
    /// Amount of tokens to transfer (no decimals)
    amount: U256,
    /// Amount to reward the node (no decimals)
    reward: U256,
    /// Acknowledgement URL to post execution receipt to
    #[arg(short, long)]
    ack_url: Option<Url>,
}

impl From<SignalArgs> for Signal {
    fn from(val: SignalArgs) -> Self {
        Signal {
            escrow_contract: val.escrow,
            token_contract: val.token,
            recipient: val.recipient,
            transfer_amount: val.amount,
            reward_amount: val.reward,
            acknowledgement_url: String::new(),
            selector_mapping: None,
        }
    }
}

impl SignalArgs {
    pub async fn execute(self, client: HttpClient) -> Result<()> {
        let res = client
            .signal(SignalPayload::Unencrypted(self.into()))
            .await
            .context("failed to submit signal to rpc")?;
        info!("{res}");
        Ok(())
    }
}
