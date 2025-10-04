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
    signers::local::PrivateKeySigner,
    transports::{RpcError, TransportErrorKind},
};
use opentelemetry::{global::meter_provider, metrics::Gauge, KeyValue};
use otel_instrument::tracer_name;
use tracing::{debug, info};

pub use crate::config::*;
use crate::contracts::{IUniswapV2Router02, IERC20};

mod config;
pub mod contracts;
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
    #[error("Read-only mode, no signers available")]
    ReadOnly,
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
