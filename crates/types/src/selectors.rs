use crate::HexSelector;
use alloy_primitives::{Bytes, Selector, U256};

/// Generate escrow contract selectors and mapping struct
macro_rules! impl_contract_selectors {
    ( $title:ident { $( $id:ident: $lit:expr ),* $(,)? } ) => {
        paste::paste! {
            // Generated const values
            $( pub const [< $title:upper _ $id:upper >]: alloy_primitives::Selector = alloy_primitives::fixed_bytes!($lit); )*

            /// Selector mapping struct with compile-time validation and fast lookups
            #[derive(serde::Serialize, serde::Deserialize, schemars::JsonSchema, Clone, Debug, PartialEq, Eq)]
            pub struct [< $title Mappings >] {
                $(
                    #[serde(rename = $lit)]
                    #[schemars(with = "HexSelector", description = stringify!($id))]
                    pub [< $id:lower >]: alloy_primitives::Selector
                ),*
            }

            impl [< $title Mappings >] {
                /// Get obfuscated selector for a specific function by original selector
                pub fn get_obfuscated_selector(&self, original: Selector) -> Option<Selector> {
                    match original {
                        $( [< $title:upper _ $id:upper >] => Some(self.[< $id:lower >]), )*
                        _ => None,
                    }
                }
            }

            impl Default for [< $title Mappings >] {
                fn default() -> Self {
                    Self {
                        $( [< $id:lower >]: Selector::ZERO, )*
                    }
                }
            }
        }
    };
}

// Define escrow contract selectors and mapping
impl_contract_selectors!(Escrow {
    fund: "0xa65e2cfd",
    bond: "0x9940686e",
    request_cancellation: "0x81972d00",
    resume: "0x046f7da2",
    collect: "0xe5225381",
    is_bonded: "0xcb766a56",
    withdraw: "0x3ccfd60b",
    current_reward_amount: "0x5a4fd645",
    bond_amount: "0x8bd03d0a",
    original_reward_amount: "0xd415b3f9",
    bonded_executor: "0x1aa7c0ec",
    execution_deadline: "0x33ee5f35",
    current_payment_amount: "0x80f323a7",
    total_bonds_deposited: "0xfe03a460",
    cancellation_request: "0x308657d7",
    funded: "0xf3a504f2",
});

impl EscrowMappings {
    /// Check if this mapping contains the required escrow function selectors
    pub fn validate_escrow_selectors(&self) -> Result<(), String> {
        // Check that critical functions have valid mappings
        let required = [
            ("bond", self.bond),
            ("collect", self.collect),
            ("is_bonded", self.is_bonded),
        ];

        let mut missing = Vec::new();
        for (name, selector) in required {
            if selector == Selector::ZERO {
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

pub type SelectorMapping = EscrowMappings;

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
    pub selector_mapping: EscrowMappings,
}

impl ObfuscatedCaller {
    pub fn new(selector_mapping: EscrowMappings) -> Self {
        Self { selector_mapping }
    }

    /// Check if contract should use obfuscated calls
    pub fn is_obfuscated(&self) -> bool {
        self.selector_mapping.bond != Selector::ZERO
            || self.selector_mapping.collect != Selector::ZERO
            || self.selector_mapping.is_bonded != Selector::ZERO
    }

    /// Prepare is_bonded() call data using obfuscated selector
    pub fn is_bonded_call_data(&self) -> Result<Bytes, String> {
        let selector = self.selector_mapping.is_bonded;
        if selector == Selector::ZERO {
            return Err("Missing is_bonded selector mapping".to_string());
        }

        let call_data = selector.to_vec(); // is_bonded() has no parameters
        Ok(call_data.into())
    }

    /// Prepare bond(uint256) call data using obfuscated selector
    pub fn bond_call_data(&self, bond_amount: U256) -> Result<Bytes, String> {
        let selector = self.selector_mapping.bond;
        if selector == Selector::ZERO {
            return Err("Missing bond selector mapping".to_string());
        }

        let mut call_data = selector.to_vec();

        // Encode uint256 parameter (32 bytes, big-endian)
        let amount_bytes = bond_amount.to_be_bytes::<32>();
        call_data.extend_from_slice(&amount_bytes);

        Ok(call_data.into())
    }

    /// Prepare collect() call data using obfuscated selector
    pub fn collect_call_data(&self) -> Result<Bytes, String> {
        let selector = self.selector_mapping.collect;
        if selector == Selector::ZERO {
            return Err("Missing collect selector mapping".to_string());
        }

        let call_data = selector.to_vec(); // collect() has no parameters
        Ok(call_data.into())
    }

    /// Parse boolean result from is_bonded call
    pub fn parse_bool_result(&self, result: &[u8]) -> bool {
        // Boolean results are returned as 32 bytes with the last byte containing the bool value
        result.len() >= 32 && result[31] != 0
    }
}
