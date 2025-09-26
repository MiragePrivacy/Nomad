# Nomad DCAP Quote

Simple bindings to `dcap_quoteprov` dynamic library for DCAP ECDSA-based remote attestations.

Specifically provides a safe implementation for generating collateral for quotes from aesmd.

```rs
let response = aesm_client
    .get_quote_ex(ecdsa_key_id, report.as_ref().to_vec(), None, vec![0; 16])?;
let collat = dcap_quote::SgxQlQveCollateral::new(&response.quote().to_vec())?;
```
