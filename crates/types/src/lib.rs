use alloy::{primitives::{Address, U256}, sol};
use serde::{Deserialize, Serialize};

#[derive(Deserialize, Serialize, Clone)]
pub struct Signal {
    pub escrow_contract: Address,
    pub token_contract: Address,
    pub recipient: Address,
    pub transfer_amount: U256,
    pub reward_amount: U256,
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
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ProcessSignalStatus {
    Processed,
    Broadcast,
}
