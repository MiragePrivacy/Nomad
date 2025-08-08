use clap::Parser;

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
pub(crate) struct Args {
    #[arg(short, long, help = "Port for the RPC server")]
    pub rpc_port: Option<u16>,

    #[arg(short, long, help = "Port for the P2P node")]
    pub p2p_port: Option<u16>,

    #[arg(help = "Multiaddr of a peer to connect to")]
    pub peer: Option<String>,

    #[arg(long, help = "Private key 1 to use")]
    pub pk1: Option<String>,

    #[arg(long, help = "Private key 2 to use")]
    pub pk2: Option<String>,

    #[arg(
        long,
        help = "Use the faucet functionality on the given token contract. For testing mode."
    )]
    pub faucet: Option<String>,

    #[arg(long, help = "HTTP RPC URL for sending transactions")]
    pub http_rpc: String,
}