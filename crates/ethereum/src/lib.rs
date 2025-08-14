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
use serde::{Deserialize, Serialize};

use nomad_types::Signal;
use tracing::debug;

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

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct EthConfig {
    pub rpc: String,
}

impl Default for EthConfig {
    fn default() -> Self {
        Self {
            rpc: "https://ethereum-rpc.publicnode.com".into(),
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
    #[error("RPC Error: {_0:?}")]
    Rpc(#[from] RpcError<TransportErrorKind>),
    #[error("Contract call failed: {_0:?}")]
    Contract(#[from] alloy::contract::Error),
    #[error("Failed to watch pending transaction: {_0}")]
    Pending(#[from] alloy::providers::PendingTransactionError),
    #[error("Failed to generate proof: {_0}")]
    Proof(#[from] proof::ProofError),
    #[error("Contract already bonded")]
    AlreadyBonded,
    #[error("Invalid contract bytecode")]
    InvalidBytecode,
    #[error("Read-only mode, no signers available")]
    ReadOnly,
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
            .connect(&config.rpc)
            .await?;
        Ok(Self { provider, accounts })
    }

    /// Faucet tokens from a given contract into each ethereum account
    pub async fn faucet(&self, contract: Address) -> Result<(), ClientError> {
        let token = TokenContract::new(contract, &self.provider);

        // Execute mint transactions and add their futures to the set
        let mut futs = Vec::new();
        for account in self.accounts.clone() {
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
        obfuscated: Vec<u8>,
    ) -> Result<(), ClientError> {
        // Ensure expected bytecode matches on-chain bytecode
        let bytecode = self.provider.get_code_at(signal.escrow_contract).await?;
        if bytecode != obfuscated {
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
        let escrow = Escrow::new(signal.escrow_contract, &self.provider);

        // Double check escrow contract is not bonded yet
        if escrow.is_bonded().call().await? {
            return Err(ClientError::AlreadyBonded);
        }

        // Compute minimum bond amount
        let bond_amount = signal
            .reward_amount
            .checked_mul(U256::from(52))
            .unwrap()
            .checked_div(U256::from(100))
            .unwrap();

        // Approve bond amount for escrow contract, on the token contract
        let approve = TokenContract::new(signal.token_contract, &self.provider)
            .approve(signal.escrow_contract, bond_amount)
            .from(self.accounts[eoa_1])
            .send()
            .await?
            .get_receipt()
            .await?;

        // Send bond call to escrow contract
        let bond = escrow
            .bond(bond_amount)
            .from(self.accounts[eoa_1])
            .send()
            .await?
            .get_receipt()
            .await?;

        // TODO: revert approval if bond failed for any reason

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
