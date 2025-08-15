use std::fmt::Debug;

use alloy::{
    network::EthereumWallet,
    primitives::{Address, U256},
    providers::{
        fillers::{
            BlobGasFiller, ChainIdFiller, FillProvider, GasFiller, JoinFill, NonceFiller,
            WalletFiller,
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
use tracing::{debug, info};
use url::Url;

mod proof;

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

#[derive(Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct EthConfig {
    pub rpc: Url,
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
        }
    }
}

type BaseProvider = FillProvider<
    JoinFill<
        JoinFill<
            Identity,
            JoinFill<GasFiller, JoinFill<BlobGasFiller, JoinFill<NonceFiller, ChainIdFiller>>>,
        >,
        WalletFiller<EthereumWallet>,
    >,
    RootProvider,
>;

pub struct EthClient {
    provider: BaseProvider,
    accounts: Vec<Address>,
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
    #[error("Contract call failed: {_0}")]
    ContractCall(String),
    #[error("Invalid selector mapping: {_0}")]
    InvalidSelectorMapping(String),
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
            .connect(config.rpc.as_str())
            .await?;
        Ok(Self { provider, accounts })
    }

    /// Faucet tokens from a given contract into each ethereum account
    pub async fn faucet(&self, contract: Address) -> Result<(), ClientError> {
        let token = TokenContract::new(contract, &self.provider);

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
                .map_err(ClientError::ContractCall)?;

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

    /// Select ideal accounts for EOA 1 and 2
    pub async fn select_accounts(&self, _signal: Signal) -> Result<[usize; 2], ClientError> {
        if self.accounts.is_empty() {
            return Err(ClientError::ReadOnly);
        }

        // TODO:
        //   - Get token balances for each account
        //   - EOA1 needs at least bond amount, EOA2 needs at least transfer amount
        //   - Error if we dont have enough balance

        Ok([0, 1])
    }

    /// Execute a bond call on the escrow contract
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
        let approve = TokenContract::new(signal.token_contract, &self.provider)
            .approve(signal.escrow_contract, bond_amount)
            .from(self.accounts[eoa_1])
            .send()
            .await?
            .get_receipt()
            .await?;

        if let Some(ref selector_mapping) = signal.selector_mapping {
            // Obfuscated contract - use raw call with obfuscated selector
            info!("Bonding to obfuscated escrow contract");

            let caller = ObfuscatedCaller::new(selector_mapping.clone());
            let call_data = caller
                .bond_call_data(bond_amount)
                .map_err(ClientError::ContractCall)?;

            let bond_receipt = self
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

            info!("Successfully bonded to obfuscated escrow");
            return Ok((approve, bond_receipt));
        }

        // Standard contract call for non-obfuscated contracts
        let escrow = Escrow::new(signal.escrow_contract, &self.provider);

        // Double check escrow contract is not bonded yet
        if escrow.is_bonded().call().await? {
            return Err(ClientError::AlreadyBonded);
        }

        // Send bond call to escrow contract
        let bond = escrow
            .bond(bond_amount)
            .from(self.accounts[eoa_1])
            .send()
            .await?
            .get_receipt()
            .await?;

        Ok((approve, bond))
    }

    /// Construct and execute a transfer call from the signal
    pub async fn transfer(
        &self,
        eoa_2: usize,
        signal: Signal,
    ) -> Result<(TransactionReceipt, proof::ProofBlob), ClientError> {
        let receipt = TokenContract::new(signal.token_contract, &self.provider)
            .transfer(signal.recipient, signal.transfer_amount)
            .from(self.accounts[eoa_2])
            .send()
            .await?
            .get_receipt()
            .await?;

        let proof = self
            .generate_proof(
                receipt.block_hash.unwrap(),
                receipt.transaction_index.unwrap(),
                None,
            )
            .await?;

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
                .map_err(ClientError::ContractCall)?;

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
