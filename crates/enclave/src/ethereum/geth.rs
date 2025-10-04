//! # GETH RA-TLS Client
//!
//! Connects directly to a GETH Enclave and provides RPC client implementation for our EOAs
//! - Obfuscated contract write calls
//! - Balance checks (ETH, ERC20)
//! - Hidden transaction polling (buildernet wont publish until its in a block)
//! - Transaction Receipts

use std::net::SocketAddr;

use alloy_rpc_types_eth::{Block, TransactionReceipt};
use alloy_sol_types::SolCall;
use color_eyre::{eyre::Context, Result};
use nomad_types::primitives::{Address, Bytes, TxHash, U256};

use serde_json::json;

use super::contracts::{Escrow, IERC20};
use super::rpc::RpcClient;

pub struct GethClient {
    rpc: RpcClient,
}

impl GethClient {
    pub fn new(rpc_addr: SocketAddr, rpc_url: String) -> Result<Self> {
        // TODO: Prefetch certificate over atls
        Ok(Self {
            rpc: RpcClient::new(rpc_url, rpc_addr, None),
        })
    }

    pub fn eth_balance_of(&self, account: Address) -> Result<U256> {
        let balance: String = self
            .rpc
            .call(
                "eth_getBalance",
                vec![format!("{:?}", account), "latest".to_string()],
            )
            .context("Failed to get ETH balance")?;

        U256::from_str_radix(balance.trim_start_matches("0x"), 16)
            .context("Failed to parse balance")
    }

    pub fn get_transaction_receipt(&self, hash: TxHash) -> Option<TransactionReceipt> {
        self.rpc
            .call("eth_getTransactionReceipt", vec![json!(hash)])
            .ok()
            .flatten()
    }

    /// Get nonce for an account
    pub fn get_transaction_count(&self, account: Address) -> Result<u64> {
        let nonce: String = self
            .rpc
            .call(
                "eth_getTransactionCount",
                vec![format!("{:?}", account), "latest".to_string()],
            )
            .context("Failed to get transaction count")?;
        u64::from_str_radix(nonce.trim_start_matches("0x"), 16).context("Failed to parse nonce")
    }

    /// Get current gas price
    pub fn gas_price(&self) -> Result<U256> {
        let gas_price: String = self
            .rpc
            .call("eth_gasPrice", json!([]))
            .context("Failed to get gas price")?;

        U256::from_str_radix(gas_price.trim_start_matches("0x"), 16)
            .context("Failed to parse gas price")
    }

    /// Estimate gas for a transaction
    pub fn estimate_gas(&self, from: Address, to: Address, data: Bytes) -> Result<U256> {
        self.rpc
            .call(
                "eth_estimateGas",
                vec![
                    json!({
                        "from": from.to_string(),
                        "to": to.to_string(),
                        "data": data.to_string(),
                    }),
                    json!("pending"),
                ],
            )
            .context("Failed to estimate gas")
    }

    /// Call contract view functions
    pub fn eth_call<C: SolCall>(&self, to: Address, data: C) -> Result<C::Return> {
        let result = self
            .rpc
            .call::<Bytes>(
                "eth_call",
                vec![
                    json!({
                        "to": to.to_string(),
                        "data": Bytes::from(data.abi_encode()).to_string(),
                    }),
                    json!("latest"),
                ],
            )
            .context("Failed to call eth_call")?;
        C::abi_decode_returns(&result).context("failed to decode response abi")
    }

    /// Get erc20 balance for contract
    pub fn erc20_balance_of(&self, token: Address, account: Address) -> Result<U256> {
        self.eth_call(token, IERC20::balanceOfCall(account))
    }

    /// Get erc20 decimals
    pub fn _erc20_decimals(&self, token: Address) -> Result<u8> {
        self.eth_call(token, IERC20::decimalsCall {})
    }

    /// Check if an escrow is bonded
    pub fn escrow_is_bonded(&self, escrow: Address) -> Result<bool> {
        self.eth_call(escrow, Escrow::is_bondedCall {})
    }

    /// Check if an escrow is funded
    pub fn escrow_is_funded(&self, escrow: Address) -> Result<bool> {
        self.eth_call(escrow, Escrow::fundedCall)
    }

    /// Get chain ID
    pub fn get_chain_id(&self) -> Result<u64> {
        let chain_id: String = self
            .rpc
            .call("eth_chainId", json!([]))
            .context("Failed to get chain ID")?;
        u64::from_str_radix(chain_id.trim_start_matches("0x"), 16)
            .context("Failed to parse chain ID")
    }

    /// Get block by hash
    pub fn get_block_by_hash(&self, hash: TxHash) -> Result<Block> {
        self.rpc
            .call(
                "eth_getBlockByHash",
                vec![json!(format!("{:?}", hash)), json!(false)],
            )
            .context("Failed to get block by hash")
    }

    /// Get all receipts for a block
    pub fn get_block_receipts(&self, block_hash: TxHash) -> Result<Vec<TransactionReceipt>> {
        self.rpc
            .call("eth_getBlockReceipts", vec![format!("{:?}", block_hash)])
            .context("Failed to get block receipts")
    }
}
