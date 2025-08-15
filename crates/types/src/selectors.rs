use alloy::primitives::{Bytes, FixedBytes, Selector, U256};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct EscrowSelectors {
    pub fund: Selector,                   // 0xa65e2cfd
    pub bond: Selector,                   // 0x9940686e
    pub request_cancellation: Selector,   // 0x81972d00
    pub resume: Selector,                 // 0x046f7da2
    pub collect: Selector,                // 0xe5225381
    pub is_bonded: Selector,              // 0xcb766a56
    pub withdraw: Selector,               // 0x3ccfd60b
    pub current_reward_amount: Selector,  // 0x5a4fd645
    pub bond_amount: Selector,            // 0x8bd03d0a
    pub original_reward_amount: Selector, // 0xd415b3f9
    pub bonded_executor: Selector,        // 0x1aa7c0ec
    pub execution_deadline: Selector,     // 0x33ee5f35
    pub current_payment_amount: Selector, // 0x80f323a7
    pub total_bonds_deposited: Selector,  // 0xfe03a460
    pub cancellation_request: Selector,   // 0x308657d7
    pub funded: Selector,                 // 0xf3a504f2
}

impl EscrowSelectors {
    pub fn new() -> Self {
        Self {
            fund: selector_from_hex("0xa65e2cfd"),
            bond: selector_from_hex("0x9940686e"),
            request_cancellation: selector_from_hex("0x81972d00"),
            resume: selector_from_hex("0x046f7da2"),
            collect: selector_from_hex("0xe5225381"),
            is_bonded: selector_from_hex("0xcb766a56"),
            withdraw: selector_from_hex("0x3ccfd60b"),
            current_reward_amount: selector_from_hex("0x5a4fd645"),
            bond_amount: selector_from_hex("0x8bd03d0a"),
            original_reward_amount: selector_from_hex("0xd415b3f9"),
            bonded_executor: selector_from_hex("0x1aa7c0ec"),
            execution_deadline: selector_from_hex("0x33ee5f35"),
            current_payment_amount: selector_from_hex("0x80f323a7"),
            total_bonds_deposited: selector_from_hex("0xfe03a460"),
            cancellation_request: selector_from_hex("0x308657d7"),
            funded: selector_from_hex("0xf3a504f2"),
        }
    }
}

impl Default for EscrowSelectors {
    fn default() -> Self {
        Self::new()
    }
}

fn selector_from_hex(hex: &str) -> Selector {
    let hex_clean = hex.trim_start_matches("0x");
    let bytes = hex::decode(hex_clean).expect("Valid hex selector");
    Selector::from(FixedBytes::from([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

/// Stores mapping between original and obfuscated selectors
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SelectorMapping {
    /// Map from original selector hex to obfuscated selector hex
    /// e.g., "0x80f323a7" -> "0x12345678"
    pub mapping: HashMap<String, String>,
}

impl SelectorMapping {
    pub fn new() -> Self {
        Self {
            mapping: HashMap::new(),
        }
    }

    /// Get obfuscated selector for a function
    pub fn get_obfuscated_selector(&self, original_hex: &str) -> Option<Selector> {
        self.mapping
            .get(original_hex)
            .map(|obfuscated_hex| selector_from_hex(obfuscated_hex))
    }

    /// Get obfuscated selector for bond function
    pub fn bond_selector(&self) -> Option<Selector> {
        self.get_obfuscated_selector("0x9940686e")
    }

    /// Get obfuscated selector for collect function  
    pub fn collect_selector(&self) -> Option<Selector> {
        self.get_obfuscated_selector("0xe5225381")
    }

    /// Get obfuscated selector for is_bonded function
    pub fn is_bonded_selector(&self) -> Option<Selector> {
        self.get_obfuscated_selector("0xcb766a56")
    }

    /// Get obfuscated selector for fund function
    pub fn fund_selector(&self) -> Option<Selector> {
        self.get_obfuscated_selector("0xa65e2cfd")
    }

    /// Check if this mapping contains the required escrow function selectors
    pub fn validate_escrow_selectors(&self) -> Result<(), String> {
        let required_functions = [
            ("bond", "0x9940686e"),
            ("collect", "0xe5225381"),
            ("is_bonded", "0xcb766a56"),
        ];

        let mut missing = Vec::new();
        for (name, selector) in required_functions {
            if !self.mapping.contains_key(selector) {
                missing.push(name);
            }
        }

        if !missing.is_empty() {
            return Err(format!(
                "Missing required selector mappings: {}",
                missing.join(", ")
            ));
        }

        Ok(())
    }
}

impl Default for SelectorMapping {
    fn default() -> Self {
        Self::new()
    }
}

/// Make raw calls with obfuscated selectors
///
/// We manually construct call data for obfuscated contracts because Alloy's
/// generated bindings can't handle runtime selector changes.
///
/// When using standard contracts, Alloy automatically handles call data construction:
/// ```rust,ignore
/// // When you write this:
/// let escrow = Escrow::new(address, &provider);
/// escrow.bond(U256::from(1000)).send().await?;
///
/// // Alloy automatically:
/// // 1. Takes the function signature: bond(uint256)
/// // 2. Computes the selector: keccak256("bond(uint256)")[0:4] = 0x9940686e
/// // 3. Encodes parameters: 1000 → [0x00, 0x00, ..., 0x03, 0xe8] (32 bytes)
/// // 4. Combines: [0x99, 0x40, 0x68, 0x6e] + [0x00, ..., 0x03, 0xe8]
/// // 5. Sends the transaction
/// ```
///
/// But for obfuscated contracts, the selectors are different at runtime, so we must
/// manually construct the call data with the correct obfuscated selectors.
pub struct ObfuscatedCaller {
    pub selector_mapping: SelectorMapping,
}

impl ObfuscatedCaller {
    pub fn new(selector_mapping: SelectorMapping) -> Self {
        Self { selector_mapping }
    }

    /// Check if contract should use obfuscated calls
    pub fn is_obfuscated(&self) -> bool {
        !self.selector_mapping.mapping.is_empty()
    }

    /// Prepare is_bonded() call data using obfuscated selector
    pub fn is_bonded_call_data(&self) -> Result<Bytes, String> {
        let selector = self
            .selector_mapping
            .is_bonded_selector()
            .ok_or("Missing is_bonded selector mapping")?;

        let call_data = selector.to_vec();
        Ok(call_data.into())
    }

    /// Prepare bond(uint256) call data using obfuscated selector
    pub fn bond_call_data(&self, bond_amount: U256) -> Result<Bytes, String> {
        let selector = self
            .selector_mapping
            .bond_selector()
            .ok_or("Missing bond selector mapping")?;

        let mut call_data = selector.to_vec();

        // Encode uint256 parameter (32 bytes, big-endian)
        let amount_bytes = bond_amount.to_be_bytes::<32>();
        call_data.extend_from_slice(&amount_bytes);

        Ok(call_data.into())
    }

    /// Prepare collect() call data using obfuscated selector
    pub fn collect_call_data(&self) -> Result<Bytes, String> {
        let selector = self
            .selector_mapping
            .collect_selector()
            .ok_or("Missing collect selector mapping")?;

        let call_data = selector.to_vec(); // collect() has no parameters
        Ok(call_data.into())
    }

    /// Parse boolean result from is_bonded call
    pub fn parse_bool_result(&self, result: &[u8]) -> bool {
        // Boolean results are returned as 32 bytes with the last byte containing the bool value
        result.len() >= 32 && result[31] != 0
    }
}
