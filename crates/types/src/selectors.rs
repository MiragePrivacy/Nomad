use alloy::hex::FromHex;
use alloy::primitives::{Bytes, FixedBytes, Selector, U256};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub const ESCROW_FUND: Selector = Selector::new([0xa6, 0x5e, 0x2c, 0xfd]);
pub const ESCROW_BOND: Selector = Selector::new([0x99, 0x40, 0x68, 0x6e]);
pub const ESCROW_REQUEST_CANCELLATION: Selector = Selector::new([0x81, 0x97, 0x2d, 0x00]);
pub const ESCROW_RESUME: Selector = Selector::new([0x04, 0x6f, 0x7d, 0xa2]);
pub const ESCROW_COLLECT: Selector = Selector::new([0xe5, 0x22, 0x53, 0x81]);
pub const ESCROW_IS_BONDED: Selector = Selector::new([0xcb, 0x76, 0x6a, 0x56]);
pub const ESCROW_WITHDRAW: Selector = Selector::new([0x3c, 0xcf, 0xd6, 0x0b]);
pub const ESCROW_CURRENT_REWARD_AMOUNT: Selector = Selector::new([0x5a, 0x4f, 0xd6, 0x45]);
pub const ESCROW_BOND_AMOUNT: Selector = Selector::new([0x8b, 0xd0, 0x3d, 0x0a]);
pub const ESCROW_ORIGINAL_REWARD_AMOUNT: Selector = Selector::new([0xd4, 0x15, 0xb3, 0xf9]);
pub const ESCROW_BONDED_EXECUTOR: Selector = Selector::new([0x1a, 0xa7, 0xc0, 0xec]);
pub const ESCROW_EXECUTION_DEADLINE: Selector = Selector::new([0x33, 0xee, 0x5f, 0x35]);
pub const ESCROW_CURRENT_PAYMENT_AMOUNT: Selector = Selector::new([0x80, 0xf3, 0x23, 0xa7]);
pub const ESCROW_TOTAL_BONDS_DEPOSITED: Selector = Selector::new([0xfe, 0x03, 0xa4, 0x60]);
pub const ESCROW_CANCELLATION_REQUEST: Selector = Selector::new([0x30, 0x86, 0x57, 0xd7]);
pub const ESCROW_FUNDED: Selector = Selector::new([0xf3, 0xa5, 0x04, 0xf2]);

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

    /// Get obfuscated selector by original selector constant
    pub fn get_obfuscated_selector(&self, original: Selector) -> Option<Selector> {
        let original_hex = format!("0x{:08x}", u32::from_be_bytes(original.into()));
        self.mapping
            .get(&original_hex)
            .and_then(|obfuscated_hex| FixedBytes::from_hex(obfuscated_hex).ok())
            .map(|bytes| Selector::new(*bytes))
    }

    /// Get obfuscated selector for bond function
    pub fn bond_selector(&self) -> Option<Selector> {
        self.get_obfuscated_selector(ESCROW_BOND)
    }

    /// Get obfuscated selector for collect function  
    pub fn collect_selector(&self) -> Option<Selector> {
        self.get_obfuscated_selector(ESCROW_COLLECT)
    }

    /// Get obfuscated selector for is_bonded function
    pub fn is_bonded_selector(&self) -> Option<Selector> {
        self.get_obfuscated_selector(ESCROW_IS_BONDED)
    }

    /// Get obfuscated selector for fund function
    pub fn fund_selector(&self) -> Option<Selector> {
        self.get_obfuscated_selector(ESCROW_FUND)
    }

    /// Check if this mapping contains the required escrow function selectors
    pub fn validate_escrow_selectors(&self) -> Result<(), String> {
        let required_functions = [
            ("bond", ESCROW_BOND),
            ("collect", ESCROW_COLLECT),
            ("is_bonded", ESCROW_IS_BONDED),
        ];

        let mut missing = Vec::new();
        for (name, selector) in required_functions {
            let selector_hex = format!("0x{:08x}", u32::from_be_bytes(selector.into()));
            if !self.mapping.contains_key(&selector_hex) {
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
/// Alloy automatically:
/// 1. Takes the function signature: bond(uint256)
/// 2. Computes the selector: keccak256("bond(uint256)")[0:4] = 0x9940686e
/// 3. Encodes parameters: 1000 → [0x00, 0x00, ..., 0x03, 0xe8] (32 bytes)
/// 4. Combines: [0x99, 0x40, 0x68, 0x6e] + [0x00, ..., 0x03, 0xe8]
/// 5. Sends the transaction
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
