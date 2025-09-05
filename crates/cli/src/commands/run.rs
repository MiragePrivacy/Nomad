use alloy::{primitives::Address, signers::local::PrivateKeySigner};
use clap::Parser;
use color_eyre::eyre::Result;
use reqwest::Url;
use tracing::info;

use nomad_node::{config::Config, NomadNode};

#[derive(Parser)]
pub struct RunArgs {
    /// Port for the api server
    #[arg(short, long)]
    pub api_port: Option<u16>,
    /// Port for the p2p node
    #[arg(short, long)]
    pub p2p_port: Option<u16>,
    /// Multiaddr of a peer to connect to
    #[arg(long)]
    pub peer: Option<String>,
    /// ETH RPC URL for sending transactions
    #[arg(long, env("ETH_RPC"))]
    pub eth_rpc: Option<Url>,
    /// Override Uniswap V2 router address
    #[arg(long)]
    pub uniswap_router: Option<Address>,
}

impl RunArgs {
    pub async fn execute(self, mut config: Config, signers: Vec<PrivateKeySigner>) -> Result<()> {
        // Apply argument overrides to configuration
        if let Some(rpc) = self.eth_rpc.clone() {
            config.eth.rpc = rpc;
        }
        if let Some(port) = self.api_port {
            config.api.port = port;
        }
        if let Some(port) = self.p2p_port {
            config.p2p.tcp = port;
        }
        if let Some(peer) = self.peer.clone() {
            config.p2p.bootstrap = vec![peer.parse().unwrap()];
        }
        if let Some(router_address) = self.uniswap_router {
            config.eth.uniswap.router = router_address;
            info!("Using Uniswap router override: {}", router_address);
        }

        NomadNode::init(config, signers).await?.run().await
    }
}
