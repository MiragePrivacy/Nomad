use color_eyre::eyre::ContextCompat;
use env_logger::Env;
use nomad_enclave::Enclave;

pub fn main() -> color_eyre::Result<()> {
    color_eyre::install()?;
    let mut args = std::env::args().skip(1);
    let addr = args.next().context("Missing control socket addr arg")?;
    env_logger::init_from_env(Env::new().default_filter_or("nomad_enclave=trace,debug"));
    Enclave::init(&addr)?.run()
}
