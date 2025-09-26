//! # Keyshare Server
//!
//! Requires:
//! - the global secret
//! - quote for the global public key
//! - collateral from the local quote provider
//!
//! 1. Accept tcp connection
//! 2. Read and validate client quote:
//!   - Quoting Enclave signature is valid
//!   - MRENCLAVE and MRSIGNER matches ours
//!   - Quoting Enclave matches our QE collateral
//! 3. Encrypt global secret with ecies for the client enclave's
//!    public key in the quote
//! 4. Reply to client enclave with encrypted payload
