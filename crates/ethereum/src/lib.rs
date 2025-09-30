use std::{fmt::Debug, time::Duration};

use alloy::{
    network::EthereumWallet,
    primitives::{
        utils::{format_ether, format_units, parse_ether},
        Address, U256,
    },
    providers::{
        fillers::{BlobGasFiller, ChainIdFiller, FillProvider, GasFiller, JoinFill, NonceFiller},
        Identity, Provider, ProviderBuilder, RootProvider,
    },
    rpc::types::TransactionReceipt,
    signers::local::PrivateKeySigner,
    transports::{RpcError, TransportErrorKind},
};
use opentelemetry::{global::meter_provider, metrics::Gauge, KeyValue};
use otel_instrument::{instrument, tracer_name};
use scc::HashMap;
use tracing::{debug, info, warn};

use nomad_types::{ObfuscatedCaller, Signal};

pub use crate::config::*;
use crate::contracts::{Escrow, IUniswapV2Router02, IERC20};

mod config;
pub mod contracts;
mod proof;
mod swap;

tracer_name!("nomad");

type ReadProvider = FillProvider<
    JoinFill<
        Identity,
        JoinFill<GasFiller, JoinFill<BlobGasFiller, JoinFill<NonceFiller, ChainIdFiller>>>,
    >,
    RootProvider,
>;

#[derive(Clone)]
pub struct EthClient {
    pub read_provider: ReadProvider,
    rpc: String,
    wallet: EthereumWallet,
    accounts: Vec<Address>,
    min_eth: (U256, f64),
    config: EthConfig,
    uniswap: Option<UniswapRuntime>,
    // Track the last used EOA 2 account index per token contract address
    last_used_eoa_2: HashMap<Address, usize>,
    // OpenTelemetry metrics for balance monitoring (optional)
    balance_metrics: Option<BalanceMetrics>,
}

#[derive(Clone)]
pub struct BalanceMetrics {
    eth_balance: Gauge<f64>,
    token_balance: Gauge<f64>,
}

#[derive(Clone)]
pub struct UniswapRuntime {
    pub config: UniswapV2Config,
    pub weth_address: Address,
    pub target_eth_wei: U256,
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
    #[error("Token swap failed: {_0}")]
    SwapFailed(String),
    #[error("Insufficient token balance for swap: need {_0}, have {_1}")]
    InsufficientTokenBalance(U256, U256),
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

        let min_eth = (
            parse_ether(&config.min_eth.to_string()).unwrap(),
            config.min_eth,
        );

        let rpc = config.rpc.to_string();
        let read_provider = ProviderBuilder::new().connect(&rpc).await?;

        // Initialize Uniswap runtime data if enabled
        let uniswap = if config.uniswap.enabled {
            let target_wei = parse_ether(&config.uniswap.target_eth_amount.to_string()).unwrap();

            // Get WETH address from router contract
            let router = IUniswapV2Router02::new(config.uniswap.router, &read_provider);
            let weth = router.WETH().call().await?;

            Some(UniswapRuntime {
                config: config.uniswap.clone(),
                weth_address: weth,
                target_eth_wei: target_wei,
            })
        } else {
            None
        };

        Ok(Self {
            read_provider,
            rpc,
            wallet,
            accounts,
            min_eth,
            config,
            uniswap,
            last_used_eoa_2: HashMap::new(),
            balance_metrics: None,
        })
    }

    /// Get a provider for the current wallets
    pub async fn wallet_provider(&self) -> Result<impl Provider, ClientError> {
        let provider = ProviderBuilder::new()
            .wallet(self.wallet.clone())
            .with_simple_nonce_management()
            .connect(&self.rpc)
            .await?;
        Ok(provider)
    }

    /// Faucet tokens from a given contract into each ethereum account
    pub async fn faucet(
        &self,
        provider: impl Provider,
        contract: Address,
    ) -> Result<(), ClientError> {
        let token = IERC20::new(contract, provider);

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
    #[instrument(skip_all, err)]
    pub async fn validate_contract(&self, signal: &Signal) -> Result<(), ClientError> {
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
                .read_provider
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

        // Ensure escrow contract is not bonded yet
        let escrow = Escrow::new(signal.escrow_contract, &self.read_provider);
        if escrow.is_bonded().call().await? {
            return Err(ClientError::AlreadyBonded);
        }

        Ok(())
    }

    /// Wait for at least a given number of given accounts to have enough eth
    #[instrument(skip_all, err)]
    pub async fn wait_for_eth(&self, accounts: &[usize], need: usize) -> Result<(), ClientError> {
        for idx in accounts {
            let account = self.accounts[*idx];
            let bal = self.read_provider.get_balance(account).await?;
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
                if self.read_provider.get_balance(self.accounts[*idx]).await? >= self.min_eth.0 {
                    have += 1;
                }
            }
        }
        Ok(())
    }

    /// Get accounts above minimum eth balance, or return error if not at least 2
    #[instrument(skip_all, err)]
    async fn get_active_accounts(&self) -> Result<Vec<usize>, ClientError> {
        let mut active = Vec::new();
        let mut inactive = Vec::new();
        for (i, address) in self.accounts.iter().cloned().enumerate() {
            if self.read_provider.get_balance(address).await? >= self.min_eth.0 {
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
    #[instrument(skip_all, fields(accounts = accounts), err)]
    async fn token_balances(
        &self,
        accounts: &[usize],
        contract: Address,
    ) -> Result<Vec<(usize, U256)>, ClientError> {
        let contract = IERC20::new(contract, &self.read_provider);

        let mut bals = Vec::new();
        for idx in accounts {
            let bal = contract.balanceOf(self.accounts[*idx]).call().await?;
            bals.push((*idx, bal))
        }

        Ok(bals)
    }

    /// Select ideal accounts for EOA 1 and 2
    #[instrument(skip_all, err)]
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

        // Get the last used EOA 2 account for this token, if any
        let last_used_eoa_2 = self
            .last_used_eoa_2
            .read_async(&signal.token_contract, |_, &v| v)
            .await;

        // find eoa 1; needs enough for bond amount.
        // should have the least amount of funds for redistribution
        balances.sort();
        let eoa_1 = *balances
            .iter()
            .find(|(_, bal)| bal >= &bond_amount)
            .ok_or(ClientError::NotEnoughTokens)?;

        // find eoa 2; needs enough for escrow.
        // should have the most amount of funds for redistribution
        // but avoid reusing the last used EOA 2 account
        balances.reverse();
        let eoa_2 = *balances
            .iter()
            .find(|(i, bal)| {
                i != &eoa_1.0 && bal >= &signal.transfer_amount && Some(*i) != last_used_eoa_2
            })
            .or_else(|| {
                // If we can't find an account that wasn't last used as EOA 2, fall back to any valid account
                balances
                    .iter()
                    .find(|(i, bal)| i != &eoa_1.0 && bal >= &signal.transfer_amount)
            })
            .ok_or(ClientError::NotEnoughTokens)?;

        // Track this EOA 2 account as the last used for this token
        self.last_used_eoa_2
            .upsert_async(signal.token_contract, eoa_2.0)
            .await;

        Ok([eoa_1.0, eoa_2.0])
    }

    /// Execute a bond call on the escrow contract. Now handles obfuscated contracts.
    #[instrument(skip_all, fields(eoa_1 = self.accounts[eoa_1]), err)]
    pub async fn bond(
        &self,
        provider: impl Provider,
        eoa_1: usize,
        signal: Signal,
    ) -> Result<[TransactionReceipt; 2], ClientError> {
        // Compute minimum bond amount
        let bond_amount = signal
            .reward_amount
            .checked_mul(U256::from(52))
            .unwrap()
            .checked_div(U256::from(100))
            .unwrap();

        // Approve bond amount for escrow contract, on the token contract (always the same)
        let approve = IERC20::new(signal.token_contract, &provider)
            .approve(signal.escrow_contract, bond_amount)
            .from(self.accounts[eoa_1])
            .send()
            .await?
            .get_receipt()
            .await?;
        opentelemetry::trace::get_active_span(|span| {
            span.set_attribute(KeyValue::new(
                "tx_approve",
                approve.transaction_hash.to_string(),
            ));
        });

        // Try to bond
        let bond_result = if let Some(ref selector_mapping) = signal.selector_mapping {
            // Obfuscated contract - use raw call with obfuscated selector
            info!("Bonding to obfuscated escrow contract");

            let caller = ObfuscatedCaller::new(selector_mapping.clone());
            let call_data = caller
                .bond_call_data(bond_amount)
                .map_err(ClientError::ObfuscatedContractCall)?;

            provider
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
            let escrow = Escrow::new(signal.escrow_contract, &provider);

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
                opentelemetry::trace::get_active_span(|span| {
                    span.set_attribute(KeyValue::new(
                        "tx_bond",
                        bond_receipt.transaction_hash.to_string(),
                    ));
                });
                info!("Successfully bonded to escrow");
                Ok([approve, bond_receipt])
            }
            Err(e) => {
                warn!(
                    "Bond failed, reverting approval to prevent stuck tokens: {:?}",
                    e
                );

                // Reset approval to 0
                let _ = IERC20::new(signal.token_contract, provider)
                    .approve(signal.escrow_contract, U256::ZERO)
                    .from(self.accounts[eoa_1])
                    .send()
                    .await;

                Err(e.into())
            }
        }
    }

    /// Construct and execute a transfer call from the signal
    #[instrument(skip_all, fields(eoa_2 = self.accounts[eoa_2]), err)]
    pub async fn transfer(
        &self,
        provider: impl Provider,
        eoa_2: usize,
        signal: Signal,
    ) -> Result<TransactionReceipt, ClientError> {
        let receipt = IERC20::new(signal.token_contract, provider)
            .transfer(signal.recipient, signal.transfer_amount)
            .from(self.accounts[eoa_2])
            .send()
            .await?
            .get_receipt()
            .await?;
        opentelemetry::trace::get_active_span(|span| {
            span.set_attribute(KeyValue::new(
                "tx_transfer",
                receipt.transaction_hash.to_string(),
            ));
        });
        Ok(receipt)
    }

    /// Collect a reward by submitting proof for a signal
    #[instrument(skip_all, fields(eoa_1 = self.accounts[eoa_1]), err)]
    pub async fn collect(
        &self,
        provider: impl Provider,
        eoa_1: usize,
        signal: Signal,
        proof: Escrow::ReceiptProof,
        block: u64,
    ) -> Result<TransactionReceipt, ClientError> {
        let receipt = if let Some(ref selector_mapping) = signal.selector_mapping {
            // Obfuscated contract - use raw call with obfuscated selector
            info!("Collecting from obfuscated escrow contract");

            let caller = ObfuscatedCaller::new(selector_mapping.clone());
            let call_data = caller
                .collect_call_data()
                .map_err(ClientError::ObfuscatedContractCall)?;

            let receipt = provider
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
            receipt
        } else {
            // Standard contract call for non-obfuscated contracts
            let receipt = Escrow::new(signal.escrow_contract, provider)
                .collect(proof, U256::from(block))
                .from(self.accounts[eoa_1])
                .send()
                .await?
                .get_receipt()
                .await?;
            info!("Successfully collected from escrow");
            receipt
        };

        opentelemetry::trace::get_active_span(|span| {
            span.set_attribute(KeyValue::new(
                "tx_collect",
                receipt.transaction_hash.to_string(),
            ));
        });
        Ok(receipt)
    }

    /// Enable balance metrics for monitoring (only used when running the node)
    pub async fn enable_balance_metrics(&mut self) {
        let meter = meter_provider().meter("nomad");

        let eth_balance = meter
            .f64_gauge("eth_balance")
            .with_description("ETH balance per account")
            .build();

        let token_balance = meter
            .f64_gauge("token_balance")
            .with_description("Token balance per account and token")
            .build();

        self.balance_metrics = Some(BalanceMetrics {
            eth_balance,
            token_balance,
        });
    }

    /// Report current balances to OpenTelemetry metrics (if enabled)
    pub async fn report_balance_metrics(&self) -> Result<(), ClientError> {
        let accounts = (0..self.accounts.len()).collect::<Vec<_>>();
        self.update_account_balance_metrics(&accounts).await
    }

    /// Update balance metrics for specific accounts after transactions
    pub async fn update_account_balance_metrics(
        &self,
        accounts: &[usize],
    ) -> Result<(), ClientError> {
        let Some(ref metrics) = self.balance_metrics else {
            return Ok(());
        };

        for &account_index in accounts {
            let address = self.accounts[account_index];

            // Update ETH balance for this account
            let eth_balance = self.read_provider.get_balance(address).await?;
            let balance_eth: f64 = format_ether(eth_balance).parse().unwrap_or(0.0);

            metrics.eth_balance.record(
                balance_eth,
                &[KeyValue::new("account", address.to_string())],
            );

            // Update token balances for this account
            for (token_name, token_config) in &self.config.token {
                let token_contract = IERC20::new(token_config.address, &self.read_provider);
                let balance = token_contract.balanceOf(address).call().await?;
                let decimals = token_contract.decimals().call().await.unwrap_or(18);
                let balance_f64: f64 = format_units(balance, decimals)
                    .unwrap_or_default()
                    .parse()
                    .unwrap_or(0.0);

                metrics.token_balance.record(
                    balance_f64,
                    &[
                        KeyValue::new("token", token_name.clone()),
                        KeyValue::new("token_address", token_config.address.to_string()),
                        KeyValue::new("account", address.to_string()),
                    ],
                );
            }
        }

        Ok(())
    }

    /// Get the chain ID of the connected network
    pub async fn chain_id(&self) -> Result<u64, ClientError> {
        let chain_id = self.read_provider.get_chain_id().await?;
        Ok(chain_id)
    }
}
