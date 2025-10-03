use color_eyre::eyre::ContextCompat;
use nomad_enclave::Enclave;
use tracing_subscriber::EnvFilter;

pub fn main() -> color_eyre::Result<()> {
    println!("Enclave started");
    // Parse args
    let mut args = std::env::args().skip(1);
    let addr = args.next().context("Missing control socket addr arg")?;
    let filter = args.next().unwrap_or("info".to_string());

    // Init tracing
    tracing_subscriber::fmt()
        .compact()
        .with_env_filter(EnvFilter::new(filter))
        .init();

    // Init and run enclave
    Enclave::init(&addr)?.run()
}
