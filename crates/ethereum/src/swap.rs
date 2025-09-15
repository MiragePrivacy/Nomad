use std::{collections::HashSet, time::Duration};

use alloy::{
    primitives::{
        utils::{format_ether, format_units},
        U256,
    },
    providers::Provider,
    rpc::types::TransactionReceipt,
};
use tracing::{debug, info, trace, warn};

use crate::{
    contracts::{IUniswapV2Router02, IERC20},
    ClientError, EthClient,
};

impl EthClient {
    /// Get the swap check interval, returns None if Uniswap is disabled
    pub fn swap_check_interval(&self) -> Option<Duration> {
        self.uniswap.as_ref().map(|u| u.config.check_interval)
    }

    /// Check which accounts need ETH and have swappable tokens
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
            if eth_balance >= self.min_eth.0 {
                continue;
            }
            let eth_deficit = self.min_eth.0 - eth_balance;

            // Calculate how many multiples of target_eth_amount we need
            let multiples_needed =
                (eth_deficit + uniswap.target_eth_wei - U256::from(1)) / uniswap.target_eth_wei; // Ceiling division
            let target_eth_to_get = multiples_needed * uniswap.target_eth_wei;

            info!(
                "Account {account} needs ETH: has {}, needs {}, target to swap for: {}",
                format_ether(eth_balance),
                format_ether(self.min_eth.0),
                format_ether(target_eth_to_get)
            );

            // Check if any tokens can be swapped to get the target ETH amount
            for (token_name, token_config) in &self.config.token {
                if !token_config.swap {
                    continue;
                }

                let token = IERC20::new(token_config.address, &self.read_provider);
                let token_balance = token.balanceOf(account).call().await?;
                let token_decimals = token.decimals().call().await?;

                // If we have more than min_balance, check if we can swap enough for target ETH
                if token_balance <= token_config.min_balance {
                    continue;
                }

                let available_tokens = token_balance - token_config.min_balance;

                // Get rough estimate of tokens needed for target ETH amount
                // (We'll do exact calculation in swap_tokens_for_eth)
                let router = IUniswapV2Router02::new(uniswap.config.router, &self.read_provider);
                let path = vec![token_config.address, uniswap.weth_address];

                // Try to get quote for available tokens to see if we can get enough ETH
                let Ok(amounts_out) = router.getAmountsOut(available_tokens, path).call().await
                else {
                    continue;
                };
                let estimated_eth_output = amounts_out[1];

                // Only add to candidates if we can get at least the target ETH amount
                if estimated_eth_output >= target_eth_to_get {
                    info!(
                        "Account {account} can swap {} {token_name} tokens for up to ~{} ETH (target: {})",
                        format_units(available_tokens, token_decimals).unwrap(),
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
                        "Account {account} has {} {token_name} tokens but can only get {} ETH, need {} ETH",
                        format_units(available_tokens, token_decimals).unwrap(),
                        format_ether(estimated_eth_output),
                        format_ether(target_eth_to_get)
                    );
                }
            }
        }

        Ok(swap_candidates)
    }

    /// Execute token-to-ETH swap via Uniswap V2
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
            .token
            .get(token_name)
            .ok_or_else(|| ClientError::SwapFailed("Token not found in config".to_string()))?;

        let account = self.accounts[account_idx];

        // Get expected output amount from Uniswap
        let router = IUniswapV2Router02::new(uniswap.config.router, &provider);

        // Path: Token -> WETH -> ETH
        let path = vec![token_config.address, uniswap.weth_address];

        // Calculate exact tokens needed for target ETH amount
        let amount_to_swap = router
            .getAmountsIn(target_eth_amount, path.clone())
            .call()
            .await?[0];

        // Verify we have enough tokens available
        if amount_to_swap > max_tokens_available {
            return Err(ClientError::SwapFailed(format!(
                "Not enough tokens available: need {amount_to_swap}, have {max_tokens_available}"
            )));
        }

        // Verify we have enough tokens
        let token = IERC20::new(token_config.address, &self.read_provider);
        let balance = token.balanceOf(account).call().await?;

        if balance < amount_to_swap {
            return Err(ClientError::InsufficientTokenBalance(
                amount_to_swap,
                balance,
            ));
        }

        // Apply slippage protection - we expect to get the target ETH amount
        let min_eth_out = target_eth_amount * U256::from(100 - uniswap.config.max_slippage_percent)
            / U256::from(100);

        // Set deadline
        let deadline = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
            + uniswap.config.swap_deadline.as_secs();

        info!(
            "Swapping {amount_to_swap} {token_name} for at least {} ETH",
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
    pub async fn maintain_eth_balances(&self) -> Result<(), ClientError> {
        let provider = self.wallet_provider().await?;
        let swap_candidates = self.check_swap_conditions().await?;

        if swap_candidates.is_empty() {
            debug!("No swap opportunities found");
            return Ok(());
        }

        info!("Found {} swap opportunities", swap_candidates.len());

        // Only execute one swap per account index
        let mut success = HashSet::new();

        // Execute swaps for accounts that need ETH
        for (account_idx, token_name, max_tokens, target_eth) in swap_candidates {
            if success.contains(&account_idx) {
                continue;
            }
            match self
                .swap_tokens_for_eth(&provider, account_idx, &token_name, max_tokens, target_eth)
                .await
            {
                Ok(receipt) => {
                    info!(
                        "Successfully swapped {token_name} to {} ETH for account {}: {}",
                        format_ether(target_eth),
                        self.accounts[account_idx],
                        receipt.transaction_hash
                    );
                    success.insert(account_idx);
                }
                Err(e) => {
                    warn!(
                        "Failed to swap {token_name} to {} ETH for account {}: {e}",
                        format_ether(target_eth),
                        self.accounts[account_idx],
                    );
                }
            }
        }

        Ok(())
    }
}
