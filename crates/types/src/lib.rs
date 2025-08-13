use alloy::primitives::{Address, U256};
use serde::{Deserialize, Serialize};

#[derive(Deserialize, Serialize, Clone, PartialEq, Eq, Hash)]
pub struct Signal {
    pub escrow_contract: Address,
    pub token_contract: Address,
    pub recipient: Address,
    pub transfer_amount: U256,
    pub reward_amount: U256,
    pub acknowledgement_url: String,
}

#[derive(Deserialize, Serialize, Clone)]
pub struct ReceiptFormat {
    pub start_time: String,
    pub end_time: String,
    pub bond_transaction_hash: String,
    pub approval_transaction_hash: String,
    pub transfer_transaction_hash: String,
    pub collection_transaction_hash: String,
}

impl std::fmt::Display for Signal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "send {} tokens to {} and collect {} tokens from escrow {}",
            self.transfer_amount, self.recipient, self.reward_amount, self.escrow_contract
        )
    }
}

impl std::fmt::Debug for Signal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Signal")
            .field("Token", &self.token_contract)
            .field("Amount", &self.transfer_amount)
            .field("Reward", &self.reward_amount)
            .finish()
    }
}
