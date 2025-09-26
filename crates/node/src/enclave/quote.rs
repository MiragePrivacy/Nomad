use aesm_client::AesmClient;
use alloy::primitives::Bytes;
use eyre::{ensure, Context, ContextCompat};
use sgx_isa::{Report, Targetinfo};

pub fn get_quote_for_report(
    aesm_client: &AesmClient,
    report: &Report,
) -> eyre::Result<(Bytes, nomad_dcap_quote::SgxQlQveCollateral, Targetinfo)> {
    let key_ids = aesm_client
        .get_supported_att_key_ids()
        .context("failed to get key ids")?;
    let ecdsa_key_ids: Vec<_> = key_ids
        .into_iter()
        .filter(|id| nomad_dcap_quote::SGX_QL_ALG_ECDSA_P256 == get_algorithm_id(id))
        .collect();
    ensure!(
        ecdsa_key_ids.len() == 1,
        "Expected exactly one ECDSA attestation key, got {} key(s) instead",
        ecdsa_key_ids.len()
    );
    let ecdsa_key_id = ecdsa_key_ids[0].to_vec();
    let quote_info = aesm_client
        .init_quote_ex(ecdsa_key_id.clone())
        .context("failed to get quote info")?;
    let target_info =
        Targetinfo::try_copy_from(quote_info.target_info()).context("Invalid target info")?;

    // SECURITY: The nonce is set to 0 since it can be arbitrarily modified, and spoofing is worthless.
    //
    // Here is an excerpt from Intel code [1]
    // > The caller can request a REPORT from the QE using a supplied nonce. This will allow
    // > the enclave requesting the quote to verify the QE used to generate the quote. This
    // > makes it more difficult for something to spoof a QE and allows the app enclave to
    // > catch it earlier. But since the authenticity of the QE lies in the knowledge of the
    // > Quote signing key, such spoofing will ultimately be detected by the quote verifier.
    // > QE REPORT.ReportData = SHA256(*p_{nonce}||*p_{quote})||0x00)
    //
    // https://github.com/intel/linux-sgx/blob/26c458905b72e66db7ac1feae04b43461ce1b76f/common/inc/sgx_uae_quote_ex.h#L158
    let quote = aesm_client
        .get_quote_ex(ecdsa_key_id, report.as_ref().to_vec(), None, vec![0; 16])
        .map(|res| res.quote().to_vec())
        .context("failed to get quote for report")?;
    let collat = nomad_dcap_quote::SgxQlQveCollateral::new(&quote)?;

    Ok((quote.into(), collat, target_info))
}

fn get_algorithm_id(key_id: &[u8]) -> u32 {
    const ALGORITHM_OFFSET: usize = 154;
    let mut bytes: [u8; 4] = Default::default();
    bytes.copy_from_slice(&key_id[ALGORITHM_OFFSET..ALGORITHM_OFFSET + 4]);
    u32::from_le_bytes(bytes)
}
