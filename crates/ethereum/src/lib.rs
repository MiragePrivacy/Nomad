use alloy::{
    network::EthereumWallet,
    primitives::{Address, U256},
    providers::{Provider, ProviderBuilder},
    rpc::types::TransactionReceipt,
    signers::local::PrivateKeySigner,
    transports::{RpcError, TransportErrorKind},
};
use nomad_types::{Escrow, Signal, TokenContract};

mod proof;

pub struct EthClient<P: Provider + Clone> {
    provider: P,
    accounts: Vec<(EthereumWallet, Address)>,
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
}

impl<P: Provider + Clone> EthClient<P> {
    pub fn new(provider: P, accounts: Vec<PrivateKeySigner>) -> Self {
        let accounts = accounts
            .into_iter()
            .map(|sk| {
                let address = sk.address();
                let wallet = EthereumWallet::new(sk);
                (wallet, address)
            })
            .collect();
        Self { provider, accounts }
    }

    /// Select ideal accounts for EOA 1 and 2
    pub async fn select_accounts(&self, _signal: Signal) -> Result<[usize; 2], ClientError> {
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
        let provider = ProviderBuilder::new()
            .wallet(&self.accounts[eoa_1].0)
            .connect_provider(&self.provider);
        let escrow = Escrow::new(signal.escrow_contract, &provider);

        // Check if escrow is bonded yet
        if escrow.is_bonded().call().await? {
            return Err(ClientError::AlreadyBonded);
        }

        let bond_amount = signal
            .reward_amount
            .checked_mul(U256::from(52))
            .unwrap()
            .checked_div(U256::from(100))
            .unwrap();

        // Approve bond amount
        let approve = TokenContract::new(signal.token_contract, &provider)
            .approve(signal.escrow_contract, bond_amount)
            .send()
            .await?
            .get_receipt()
            .await?;

        // Send bond call
        let bond = escrow.bond(bond_amount).send().await?.get_receipt().await?;

        Ok((approve, bond))
    }

    /// Construct and execute a transfer call from the signal
    pub async fn transfer(
        &self,
        eoa_2: usize,
        signal: Signal,
    ) -> Result<(TransactionReceipt, proof::ProofBlob), ClientError> {
        let provider = ProviderBuilder::new()
            .wallet(&self.accounts[eoa_2].0)
            .connect_provider(&self.provider);

        let receipt = TokenContract::new(signal.token_contract, provider)
            .transfer(signal.recipient, signal.transfer_amount)
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
        let provider = ProviderBuilder::new()
            .wallet(&self.accounts[eoa_1].0)
            .connect_provider(&self.provider);

        // TODO: actually send proof
        let receipt = Escrow::new(signal.escrow_contract, provider)
            .collect()
            .send()
            .await?
            .get_receipt()
            .await?;

        Ok(receipt)
    }
}
