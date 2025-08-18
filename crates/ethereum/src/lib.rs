use std::{fmt::Debug, time::Duration};

use alloy::{
    network::EthereumWallet,
    primitives::{
        utils::{format_ether, parse_ether},
        Address, U256,
    },
    providers::{
        fillers::{
            BlobGasFiller, ChainIdFiller, FillProvider, GasFiller, JoinFill, NonceFiller,
            SimpleNonceManager, WalletFiller,
        },
        Identity, Provider, ProviderBuilder, RootProvider,
    },
    rpc::types::TransactionReceipt,
    signers::local::PrivateKeySigner,
    sol,
    transports::{RpcError, TransportErrorKind},
};
use nomad_types::{ObfuscatedCaller, Signal};
use serde::{Deserialize, Serialize};
use tracing::{debug, info, warn};
use url::Url;

mod proof;

sol! {
    #[sol(rpc)]
    contract IERC20 {
        event Transfer(address indexed from, address indexed to, uint256 value);
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

#[derive(Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct EthConfig {
    /// Url for rpc commands
    pub rpc: Url,
    /// Minimum eth required for an account to be usable
    pub min_eth: f64,
}

impl Debug for EthConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Hide potentially sensitive query parameters
        f.debug_struct("EthConfig")
            .field("rpc", &self.rpc.host_str().unwrap_or("missing rpc host"))
            .finish()
    }
}

impl Default for EthConfig {
    fn default() -> Self {
        Self {
            rpc: "https://ethereum-rpc.publicnode.com".parse().unwrap(),
            min_eth: 0.01,
        }
    }
}

type BaseProvider = FillProvider<
    JoinFill<
        JoinFill<
            JoinFill<
                Identity,
                JoinFill<GasFiller, JoinFill<BlobGasFiller, JoinFill<NonceFiller, ChainIdFiller>>>,
            >,
            WalletFiller<EthereumWallet>,
        >,
        NonceFiller<SimpleNonceManager>,
    >,
    RootProvider,
>;

pub struct EthClient {
    provider: BaseProvider,
    accounts: Vec<Address>,
    min_eth: (U256, f64),
}

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("RPC Error")]
    Rpc(#[from] RpcError<TransportErrorKind>),
    #[error("Contract call failed")]
    Contract(#[from] alloy::contract::Error),
    #[error("Failed to watch pending transaction")]
    Pending(#[from] alloy::providers::PendingTransactionError),
    #[error("Failed to generate proof")]
    Proof(#[from] proof::ProofError),
    #[error("Contract already bonded")]
    AlreadyBonded,
    #[error("Invalid contract bytecode")]
    InvalidBytecode,
    #[error("Read-only mode, no signers available")]
    ReadOnly,
    #[error("Obfuscated Contract call failed: {_0}")]
    ObfuscatedContractCall(String),
    #[error("Invalid selector mapping: {_0}")]
    InvalidSelectorMapping(String),
    #[error("Eth below minimum balance ({_0}) for the accounts: {_1:?}, need at least {_2} account funded")]
    NotEnoughEth(f64, Vec<usize>, usize),
    #[error("No accounts have enough token balance to execute the signal")]
    NotEnoughTokens,
}

impl EthClient {
    pub async fn new(
        config: EthConfig,
        accounts: Vec<PrivateKeySigner>,
    ) -> Result<Self, ClientError> {
        debug!(?config);

        let mut wallet = EthereumWallet::default();
        let accounts = accounts
            .into_iter()
            .map(|sk| {
                let address = sk.address();
                wallet.register_signer(sk);
                address
            })
            .collect();
        let provider = ProviderBuilder::new()
            .wallet(wallet)
            .with_simple_nonce_management()
            .connect(config.rpc.as_str())
            .await?;
        let min_eth = (
            parse_ether(&config.min_eth.to_string()).unwrap(),
            config.min_eth,
        );

        Ok(Self {
            provider,
            accounts,
            min_eth,
        })
    }

    /// Faucet tokens from a given contract into each ethereum account
    pub async fn faucet(&self, contract: Address) -> Result<(), ClientError> {
        let token = IERC20::new(contract, &self.provider);

        // Execute mint transactions and add their futures to the set
        let mut futs = Vec::new();
        for account in self.accounts.clone() {
            info!("Minting tokens for {account}");
            let res = token.mint().from(account).send().await?;
            futs.push(res.watch());
        }

        // Wait for all mint transactions to be verified
        for fut in futs {
            fut.await?;
        }

        Ok(())
    }

    /// Validate the escrow contract for a given signal. Checks:
    /// - bytecode on-chain should match expected obfuscation output
    /// - escrow contract is not bonded yet
    pub async fn validate_contract(
        &self,
        signal: Signal,
        expected_bytecode: Vec<u8>,
    ) -> Result<(), ClientError> {
        if let Some(ref selector_mapping) = signal.selector_mapping {
            // This is an obfuscated contract
            info!(
                "Validating obfuscated escrow contract at {}",
                signal.escrow_contract
            );

            // Validate selector mapping has required functions
            selector_mapping
                .validate_escrow_selectors()
                .map_err(ClientError::InvalidSelectorMapping)?;

            let caller = ObfuscatedCaller::new(selector_mapping.clone());

            // Check if contract is already bonded using obfuscated selector
            let call_data = caller
                .is_bonded_call_data()
                .map_err(ClientError::ObfuscatedContractCall)?;

            let result = self
                .provider
                .call(alloy::rpc::types::TransactionRequest {
                    to: Some(alloy::primitives::TxKind::Call(signal.escrow_contract)),
                    input: call_data.into(),
                    ..Default::default()
                })
                .await?;

            if caller.parse_bool_result(&result) {
                return Err(ClientError::AlreadyBonded);
            }

            info!("Obfuscated contract validation successful");
            return Ok(());
        }

        // Standard validation for non-obfuscated contracts
        let bytecode = self.provider.get_code_at(signal.escrow_contract).await?;
        if bytecode != expected_bytecode {
            return Err(ClientError::InvalidBytecode);
        }

        // Ensure escrow contract is not bonded yet
        let escrow = Escrow::new(signal.escrow_contract, &self.provider);
        if escrow.is_bonded().call().await? {
            return Err(ClientError::AlreadyBonded);
        }

        Ok(())
    }

    /// Wait for at least a given number of given accounts to have enough eth
    pub async fn wait_for_eth(&self, accounts: &[usize], need: usize) -> Result<(), ClientError> {
        for idx in accounts {
            let account = self.accounts[*idx];
            let bal = self.provider.get_balance(account).await?;
            let required = self.min_eth.0 - bal;
            warn!(
                ?account,
                balance = format_ether(bal),
                "Waiting for at least {} ETH",
                format_ether(required)
            );
        }

        let mut have = 0;
        while have < need {
            tokio::time::sleep(Duration::from_secs(5 * 60)).await;
            have = 0;
            for idx in accounts {
                if self.provider.get_balance(self.accounts[*idx]).await? >= self.min_eth.0 {
                    have += 1;
                }
            }
        }
        Ok(())
    }

    /// Get accounts above minimum eth balance, or return error if not at least 2
    async fn get_active_accounts(&self) -> Result<Vec<usize>, ClientError> {
        let mut active = Vec::new();
        let mut inactive = Vec::new();
        for (i, address) in self.accounts.iter().cloned().enumerate() {
            if self.provider.get_balance(address).await? >= self.min_eth.0 {
                active.push(i);
            } else {
                inactive.push(i);
            }
        }
        if active.len() < 2 {
            return Err(ClientError::NotEnoughEth(
                self.min_eth.1,
                inactive,
                2 - active.len(),
            ));
        }
        Ok(active)
    }

    /// Get contract balances
    async fn token_balances(
        &self,
        accounts: &[usize],
        contract: Address,
    ) -> Result<Vec<(usize, U256)>, ClientError> {
        let contract = IERC20::new(contract, &self.provider);

        let mut bals = Vec::new();
        for idx in accounts {
            let bal = contract.balanceOf(self.accounts[*idx]).call().await?;
            bals.push((*idx, bal))
        }

        Ok(bals)
    }

    /// Select ideal accounts for EOA 1 and 2
    pub async fn select_accounts(&self, signal: Signal) -> Result<[usize; 2], ClientError> {
        if self.accounts.is_empty() {
            return Err(ClientError::ReadOnly);
        }

        let accounts = self.get_active_accounts().await?;
        let mut balances = self
            .token_balances(&accounts, signal.token_contract)
            .await?;

        // Compute minimum bond amount
        let bond_amount = signal
            .reward_amount
            .checked_mul(U256::from(52))
            .unwrap()
            .checked_div(U256::from(100))
            .unwrap();

        // find eoa 1; needs enough for bond amount.
        // should have the least amount of funds for redistribution
        balances.sort();
        let eoa_1 = *balances
            .iter()
            .find(|(_, bal)| bal >= &bond_amount)
            .ok_or(ClientError::NotEnoughTokens)?;

        // find eoa 2; needs enough for escrow.
        // should have the most amount of funds for redistribution
        balances.reverse();
        let eoa_2 = *balances
            .iter()
            .find(|(i, bal)| i != &eoa_1.0 && bal >= &signal.transfer_amount)
            .ok_or(ClientError::NotEnoughTokens)?;

        Ok([eoa_1.0, eoa_2.0])
    }

    /// Execute a bond call on the escrow contract. Now handles obfuscated contracts.
    pub async fn bond(
        &self,
        eoa_1: usize,
        signal: Signal,
    ) -> Result<(TransactionReceipt, TransactionReceipt), ClientError> {
        // Compute minimum bond amount
        let bond_amount = signal
            .reward_amount
            .checked_mul(U256::from(52))
            .unwrap()
            .checked_div(U256::from(100))
            .unwrap();

        // Approve bond amount for escrow contract, on the token contract (always the same)
        let approve = IERC20::new(signal.token_contract, &self.provider)
            .approve(signal.escrow_contract, bond_amount)
            .from(self.accounts[eoa_1])
            .send()
            .await?
            .get_receipt()
            .await?;

        // Try to bond
        let bond_result = if let Some(ref selector_mapping) = signal.selector_mapping {
            // Obfuscated contract - use raw call with obfuscated selector
            info!("Bonding to obfuscated escrow contract");

            let caller = ObfuscatedCaller::new(selector_mapping.clone());
            let call_data = caller
                .bond_call_data(bond_amount)
                .map_err(ClientError::ObfuscatedContractCall)?;

            self.provider
                .send_transaction(alloy::rpc::types::TransactionRequest {
                    to: Some(alloy::primitives::TxKind::Call(signal.escrow_contract)),
                    input: call_data.into(),
                    from: Some(self.accounts[eoa_1]),
                    ..Default::default()
                })
                .await?
                .get_receipt()
                .await
        } else {
            // Standard contract call for non-obfuscated contracts
            let escrow = Escrow::new(signal.escrow_contract, &self.provider);

            // Double check escrow contract is not bonded yet
            if escrow.is_bonded().call().await? {
                return Err(ClientError::AlreadyBonded);
            }

            // Send bond call to escrow contract
            escrow
                .bond(bond_amount)
                .from(self.accounts[eoa_1])
                .send()
                .await?
                .get_receipt()
                .await
        };

        // If bond failed, revert approval to prevent stuck approvals
        match bond_result {
            Ok(bond_receipt) => {
                info!("Successfully bonded to escrow");
                Ok((approve, bond_receipt))
            }
            Err(e) => {
                warn!(
                    "Bond failed, reverting approval to prevent stuck tokens: {:?}",
                    e
                );

                // Reset approval to 0
                let _ = IERC20::new(signal.token_contract, &self.provider)
                    .approve(signal.escrow_contract, U256::ZERO)
                    .from(self.accounts[eoa_1])
                    .send()
                    .await;

                Err(e.into())
            }
        }
    }

    /// Construct and execute a transfer call from the signal
    pub async fn transfer(
        &self,
        eoa_2: usize,
        signal: Signal,
    ) -> Result<(TransactionReceipt, proof::ProofBlob), ClientError> {
        let receipt = IERC20::new(signal.token_contract, &self.provider)
            .transfer(signal.recipient, signal.transfer_amount)
            .from(self.accounts[eoa_2])
            .send()
            .await?
            .get_receipt()
            .await?;
        let proof = self.generate_proof(&signal, &receipt).await?;
        Ok((receipt, proof))
    }

    /// Collect a reward by submitting proof for a signal
    pub async fn collect(
        &self,
        eoa_1: usize,
        signal: Signal,
        _proof: proof::ProofBlob,
    ) -> Result<TransactionReceipt, ClientError> {
        if let Some(ref selector_mapping) = signal.selector_mapping {
            // Obfuscated contract - use raw call with obfuscated selector
            info!("Collecting from obfuscated escrow contract");

            let caller = ObfuscatedCaller::new(selector_mapping.clone());
            let call_data = caller
                .collect_call_data()
                .map_err(ClientError::ObfuscatedContractCall)?;

            let receipt = self
                .provider
                .send_transaction(alloy::rpc::types::TransactionRequest {
                    to: Some(alloy::primitives::TxKind::Call(signal.escrow_contract)),
                    input: call_data.into(),
                    from: Some(self.accounts[eoa_1]),
                    ..Default::default()
                })
                .await?
                .get_receipt()
                .await?;

            info!("Successfully collected from obfuscated escrow");
            return Ok(receipt);
        }

        // Standard contract call for non-obfuscated contracts
        // TODO: actually send proof
        let receipt = Escrow::new(signal.escrow_contract, &self.provider)
            .collect()
            .from(self.accounts[eoa_1])
            .send()
            .await?
            .get_receipt()
            .await?;

        Ok(receipt)
    }
}
