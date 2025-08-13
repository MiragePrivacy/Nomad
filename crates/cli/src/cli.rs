use clap::Parser;

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
pub(crate) struct Args {
    /// Private key #1 to use
    #[arg(long)]
    pub pk1: Option<String>,
    /// Private key #2 to use
    #[arg(long)]
    pub pk2: Option<String>,
    /// Use the faucet functionality on the given token contract. For testing mode.
    #[arg(long)]
    pub faucet: Option<String>,

    /* Config overrides */
    /// Port for the RPC server
    #[arg(short, long)]
    pub rpc_port: Option<u16>,
    /// Port for the p2p node
    #[arg(short, long)]
    pub p2p_port: Option<u16>,
    /// Multiaddr of a peer to connect to
    #[arg(long)]
    pub peer: Option<String>,
    /// HTTP RPC URL for sending transactions
    #[arg(long)]
    pub http_rpc: Option<String>,
}

