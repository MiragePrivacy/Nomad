use nomad_enclave::Enclave;

pub fn main() -> color_eyre::Result<()> {
    Enclave::init(
        &std::env::args()
            .next()
            .expect("failed to read control socket arg"),
    )?
    .run()
}
