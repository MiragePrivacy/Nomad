use nomad_enclave::Enclave;

pub fn main() -> eyre::Result<()> {
    Enclave::init(
        &std::env::args()
            .next()
            .expect("failed to read control socket arg"),
    )?
    .run()
}
