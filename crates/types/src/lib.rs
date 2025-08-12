use alloy::{
    primitives::{Address, U256},
    sol,
};
use serde::{Deserialize, Serialize};

#[derive(Deserialize, Serialize, Clone)]
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

sol! {
    #[sol(rpc)]
    contract TokenContract {
        function balanceOf(address) public view returns (uint256);
        function mint() external;
        function transfer(address to, uint256 value) external returns (bool);
        function approve(address spender, uint256 value) external returns (bool);
    }

    #[sol(rpc)]
    contract Escrow {
        function bond(uint256 _bondAmount) public;
        function collect() public;
        function is_bonded() public view returns (bool);
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ProcessSignalStatus {
    Processed,
    Broadcast,
}
