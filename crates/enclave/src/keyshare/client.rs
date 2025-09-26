//! Keyshare Client
//!
//! 1. Connect to server and send attestation containing our client key and debug mode
//! 2. Receive and validate remote attestation containing the global public key
//! 3. Receive ecies payload containing global secret
//! 4. Decrypt with client ecies and validate `secret.publickey == Quote.reportdata.publickey`
//! 5. Seal global key and send to userspace
