use std::{
    collections::HashMap,
    fmt::{Debug, Display},
    time::Duration,
};

use alloy::{
    network::EthereumWallet,
    primitives::{
        utils::{format_ether, parse_ether},
        Address, U256,
    },
    providers::{
        fillers::{BlobGasFiller, ChainIdFiller, FillProvider, GasFiller, JoinFill, NonceFiller},
        Identity, Provider, ProviderBuilder, RootProvider,
    },
    rpc::types::TransactionReceipt,
    signers::local::PrivateKeySigner,
    sol,
    transports::{RpcError, TransportErrorKind},
};
use nomad_types::{ObfuscatedCaller, Signal};
use serde::{Deserialize, Serialize};
use tracing::{debug, field::Empty, info, instrument, trace, warn, Span};
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
    contract IUniswapV2Router02 {
        function swapExactTokensForETH(
            uint amountIn,
            uint amountOutMin,
            address[] calldata path,
            address to,
            uint deadline
        ) external returns (uint[] memory amounts);
        function getAmountsOut(uint amountIn, address[] calldata path)
            external view returns (uint[] memory amounts);
        function WETH() external pure returns (address);
        function factory() external pure returns (address);
    }

    #[sol(rpc)]
    contract Escrow {
        #[derive(Deserialize, Serialize)]
        struct ReceiptProof {
            /// RLP-encoded block header
            bytes header;
            /// RLP-encoded target receipt
            bytes receipt;
            /// Serialized MPT proof nodes
            bytes proof;
            /// RLP-encoded receipt index
            bytes path;
            /// Index of target log in receipt
            uint256 log;
        }

        function bond(uint256 _bondAmount) public;
        function collect(ReceiptProof calldata proof, uint256 targetBlockNumber) public;
        function is_bonded() public view returns (bool);
    }
}

impl Display for Escrow::ReceiptProof {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&serde_json::to_string_pretty(self).unwrap())
    }
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct EthConfig {
    /// Url for rpc commands
    pub rpc: Url,
    /// Minimum eth required for an account to be usable
    pub min_eth: f64,
    /// Uniswap V2 configuration
    pub uniswap: UniswapV2Config,
    /// Token swap configuration - table keyed by name
    pub token_swaps: HashMap<String, TokenSwapConfig>,
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct UniswapV2Config {
    pub enabled: bool,
    pub router: Address,
    pub max_slippage_percent: u8,
    #[serde(with = "humantime_serde")]
    pub swap_deadline: Duration,
    pub target_eth_amount: f64,
    #[serde(with = "humantime_serde")]
    pub check_interval: Duration,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct TokenSwapConfig {
    pub address: Address,
    pub min_balance: U256,
    pub enabled: bool,
}

#[derive(Clone)]
pub struct UniswapRuntime {
    pub config: UniswapV2Config,
    pub weth_address: Address,
    pub target_eth_wei: U256,
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
        // Add default USDC configuration (mainnet)
        let mut token_swaps = HashMap::new();
        token_swaps.insert(
            "USDC".to_string(),
            TokenSwapConfig {
                address: "0xA0b86a33E6d9A77F45Ac7Be05d83c1B40c8063c5"
                    .parse()
                    .unwrap(), // Mainnet USDC
                min_balance: U256::from(1_000_000_000u64), // 1000 USDC (6 decimals)
                enabled: false,                            // Disabled by default for safety
            },
        );

        Self {
            rpc: "https://ethereum-rpc.publicnode.com".parse().unwrap(),
            min_eth: 0.01,
            uniswap: UniswapV2Config::default(),
            token_swaps,
        }
    }
}

impl Default for UniswapV2Config {
    fn default() -> Self {
        Self {
            enabled: false, // Disabled by default for safety
            router: "0x7a250d5630B4cF539739dF2C5dAcb4c659F2488D"
                .parse()
                .unwrap(), // Mainnet router
            max_slippage_percent: 5,
            swap_deadline: Duration::from_secs(20 * 60), // 20 minutes
            target_eth_amount: 0.005, // Default to swapping for 0.005 ETH at a time
            check_interval: Duration::from_secs(5 * 60), // Check every 5 minutes
        }
    }
}

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
    #[instrument(skip_all)]
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

        // Standard validation for non-obfuscated contracts
        let bytecode = self
            .read_provider
            .get_code_at(signal.escrow_contract)
            .await?;
        if bytecode != expected_bytecode {
            return Err(ClientError::InvalidBytecode);
        }

        // Ensure escrow contract is not bonded yet
        let escrow = Escrow::new(signal.escrow_contract, &self.read_provider);
        if escrow.is_bonded().call().await? {
            return Err(ClientError::AlreadyBonded);
        }

        Ok(())
    }

    /// Wait for at least a given number of given accounts to have enough eth
    #[instrument(skip_all)]
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
    #[instrument(skip_all)]
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
    #[instrument(skip_all, fields(?accounts))]
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
    #[instrument(skip_all)]
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
    #[instrument(skip_all, fields(eoa_1 = ?self.accounts[eoa_1], tx_approve = Empty, tx_bond = Empty))]
    pub async fn bond(
        &self,
        provider: impl Provider,
        eoa_1: usize,
        signal: Signal,
    ) -> Result<[TransactionReceipt; 2], ClientError> {
        let span = Span::current();
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
        span.record("tx_approve", approve.transaction_hash.to_string());

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
                span.record("tx_bond", bond_receipt.transaction_hash.to_string());
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
    #[instrument(skip_all, fields(eoa_2 = ?self.accounts[eoa_2], tx_transfer = Empty))]
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
        Span::current().record("tx_transfer", receipt.transaction_hash.to_string());
        Ok(receipt)
    }

    /// Collect a reward by submitting proof for a signal
    #[instrument(skip_all, fields(eoa_1 = ?self.accounts[eoa_1], tx_collect = Empty))]
    pub async fn collect(
        &self,
        provider: impl Provider,
        eoa_1: usize,
        signal: Signal,
        proof: Escrow::ReceiptProof,
        block: u64,
    ) -> Result<TransactionReceipt, ClientError> {
        let span = Span::current();
        if let Some(ref selector_mapping) = signal.selector_mapping {
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
            span.record("tx_collect", receipt.transaction_hash.to_string());

            info!("Successfully collected from obfuscated escrow");
            return Ok(receipt);
        }

        // Standard contract call for non-obfuscated contracts
        let receipt = Escrow::new(signal.escrow_contract, provider)
            .collect(proof, U256::from(block))
            .from(self.accounts[eoa_1])
            .send()
            .await?
            .get_receipt()
            .await?;
        span.record("tx_collect", receipt.transaction_hash.to_string());
        info!("Successfully collected from escrow");

        Ok(receipt)
    }

    /// Check which accounts need ETH and have swappable tokens
    #[instrument(skip_all)]
    pub async fn check_swap_conditions(
        &self,
    ) -> Result<Vec<(usize, String, U256, U256)>, ClientError> {
        // Early return if Uniswap is not configured
        let Some(uniswap) = self.uniswap.as_ref() else {
            trace!("Uniswap disabled, skipping swap condition checks");
            return Ok(Vec::new());
        };
        let mut swap_candidates = Vec::new();

        for (account_idx, &account) in self.accounts.iter().enumerate() {
            let eth_balance = self.read_provider.get_balance(account).await?;

            // Check if this account needs ETH
            if eth_balance < self.min_eth.0 {
                let eth_deficit = self.min_eth.0 - eth_balance;

                // Calculate how many multiples of target_eth_amount we need
                let multiples_needed =
                    (eth_deficit + uniswap.target_eth_wei - U256::from(1)) / uniswap.target_eth_wei; // Ceiling division
                let target_eth_to_get = multiples_needed * uniswap.target_eth_wei;

                info!(
                    "Account {} needs ETH: has {}, needs {}, target to swap for: {}",
                    account,
                    format_ether(eth_balance),
                    format_ether(self.min_eth.0),
                    format_ether(target_eth_to_get)
                );

                // Check if any tokens can be swapped to get the target ETH amount
                for (token_name, token_config) in &self.config.token_swaps {
                    if !token_config.enabled {
                        continue;
                    }

                    let token = IERC20::new(token_config.address, &self.read_provider);
                    let token_balance = token.balanceOf(account).call().await?;

                    // If we have more than min_balance, check if we can swap enough for target ETH
                    if token_balance > token_config.min_balance {
                        let available_tokens = token_balance - token_config.min_balance;

                        // Get rough estimate of tokens needed for target ETH amount
                        // (We'll do exact calculation in swap_tokens_for_eth)
                        let router =
                            IUniswapV2Router02::new(uniswap.config.router, &self.read_provider);
                        let path = vec![token_config.address, uniswap.weth_address];

                        // Try to get quote for available tokens to see if we can get enough ETH
                        if let Ok(amounts_out) =
                            router.getAmountsOut(available_tokens, path).call().await
                        {
                            let estimated_eth_output = amounts_out[1];

                            // Only add to candidates if we can get at least the target ETH amount
                            if estimated_eth_output >= target_eth_to_get {
                                info!(
                                    "Account {} can swap {} {} tokens for ~{} ETH (target: {})",
                                    account,
                                    available_tokens,
                                    token_name,
                                    format_ether(estimated_eth_output),
                                    format_ether(target_eth_to_get)
                                );
                                swap_candidates.push((
                                    account_idx,
                                    token_name.clone(),
                                    available_tokens,
                                    target_eth_to_get,
                                ));
                            } else {
                                debug!(
                                    "Account {} has {} {} tokens but can only get {} ETH, need {} ETH",
                                    account,
                                    available_tokens,
                                    token_name,
                                    format_ether(estimated_eth_output),
                                    format_ether(target_eth_to_get)
                                );
                            }
                        }
                    }
                }
            }
        }

        Ok(swap_candidates)
    }

    /// Execute token-to-ETH swap via Uniswap V2
    #[instrument(skip_all, fields(account = ?self.accounts[account_idx], token = token_name, target_eth = %target_eth_amount))]
    pub async fn swap_tokens_for_eth(
        &self,
        provider: impl Provider,
        account_idx: usize,
        token_name: &str,
        max_tokens_available: U256,
        target_eth_amount: U256,
    ) -> Result<TransactionReceipt, ClientError> {
        let Some(uniswap) = self.uniswap.as_ref() else {
            return Err(ClientError::SwapFailed(
                "Uniswap not configured".to_string(),
            ));
        };

        let token_config = self
            .config
            .token_swaps
            .get(token_name)
            .ok_or_else(|| ClientError::SwapFailed("Token not found in config".to_string()))?;

        let account = self.accounts[account_idx];

        // Get expected output amount from Uniswap
        let router = IUniswapV2Router02::new(uniswap.config.router, &provider);

        // Path: Token -> WETH -> ETH
        let path = vec![token_config.address, uniswap.weth_address];

        // Calculate exact tokens needed for target ETH amount
        // We'll use the maximum available tokens but verify we get at least target ETH
        let amounts_out = router
            .getAmountsOut(max_tokens_available, path.clone())
            .call()
            .await?;
        let expected_eth = amounts_out[1];

        // Verify we can get at least the target ETH amount
        if expected_eth < target_eth_amount {
            return Err(ClientError::SwapFailed(format!(
                "Cannot get enough ETH: need {}, can only get {} with {} tokens",
                format_ether(target_eth_amount),
                format_ether(expected_eth),
                max_tokens_available
            )));
        }

        let amount_to_swap = max_tokens_available;

        // Verify we have enough tokens
        let token = IERC20::new(token_config.address, &self.read_provider);
        let balance = token.balanceOf(account).call().await?;

        if balance < amount_to_swap {
            return Err(ClientError::InsufficientTokenBalance(
                amount_to_swap,
                balance,
            ));
        }

        // Apply slippage protection
        let min_eth_out =
            expected_eth * U256::from(100 - uniswap.config.max_slippage_percent) / U256::from(100);

        // Set deadline
        let deadline = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
            + uniswap.config.swap_deadline.as_secs();

        info!(
            "Swapping {} {} for at least {} ETH",
            amount_to_swap,
            token_name,
            format_ether(min_eth_out)
        );

        // First approve the router to spend tokens
        let approve_tx = token
            .approve(uniswap.config.router, amount_to_swap)
            .from(account)
            .send()
            .await?
            .get_receipt()
            .await?;

        info!(
            "Approved router to spend tokens: {}",
            approve_tx.transaction_hash
        );

        // Execute swap
        let swap_tx = router
            .swapExactTokensForETH(
                amount_to_swap,
                min_eth_out,
                path,
                account,
                U256::from(deadline),
            )
            .from(account)
            .send()
            .await?
            .get_receipt()
            .await?;

        info!("Swap completed: {}", swap_tx.transaction_hash);

        Ok(swap_tx)
    }

    /// Monitor and maintain minimum ETH balances by swapping tokens
    #[instrument(skip_all)]
    pub async fn maintain_eth_balances(&self) -> Result<(), ClientError> {
        let provider = self.wallet_provider().await?;
        let swap_candidates = self.check_swap_conditions().await?;

        if swap_candidates.is_empty() {
            debug!("No swap opportunities found");
            return Ok(());
        }

        info!("Found {} swap opportunities", swap_candidates.len());

        // Execute swaps for accounts that need ETH
        for (account_idx, token_name, max_tokens, target_eth) in swap_candidates {
            match self
                .swap_tokens_for_eth(&provider, account_idx, &token_name, max_tokens, target_eth)
                .await
            {
                Ok(receipt) => {
                    info!(
                        "Successfully swapped {} {} to ETH for account {} (target: {} ETH): {}",
                        max_tokens,
                        token_name,
                        self.accounts[account_idx],
                        format_ether(target_eth),
                        receipt.transaction_hash
                    );
                }
                Err(e) => {
                    warn!(
                        "Failed to swap {} {} for account {} (target: {} ETH): {}",
                        token_name,
                        max_tokens,
                        self.accounts[account_idx],
                        format_ether(target_eth),
                        e
                    );
                }
            }
        }

        Ok(())
    }

    /// Get the swap check interval, returns None if Uniswap is disabled
    pub fn swap_check_interval(&self) -> Option<Duration> {
        self.uniswap.as_ref().map(|u| u.config.check_interval)
    }
}
