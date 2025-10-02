//! # GETH RA-TLS Client
//!
//! Connects directly to a GETH Enclave and provides RPC client implementation for our EOAs
//! - Obfuscated contract write calls
//! - Balance checks (ETH, ERC20)
//! - Hidden transaction polling (buildernet wont publish until its in a block)
//! - Transaction Receipts

use alloy_rpc_types_eth::{Block, TransactionReceipt};
use alloy_sol_types::SolCall;
use color_eyre::{
    eyre::{eyre, Context},
    Result,
};
use nomad_types::primitives::{Address, Bytes, TxHash, U256};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tracing::trace;

use super::contracts::{Escrow, IERC20};

#[derive(Serialize)]
struct JsonRpcRequest<T> {
    jsonrpc: &'static str,
    method: &'static str,
    params: T,
    id: u64,
}

#[derive(Deserialize)]
struct JsonRpcResponse<T> {
    #[allow(dead_code)]
    jsonrpc: String,
    result: Option<T>,
    error: Option<JsonRpcError>,
    #[allow(dead_code)]
    id: u64,
}

#[derive(Deserialize, Debug)]
struct JsonRpcError {
    code: i64,
    message: String,
}

pub struct GethClient {
    rpc_url: String,
    _cert: Option<String>,
}

impl GethClient {
    pub fn new(rpc_url: String) -> Result<Self> {
        // TODO: connect to the rpc endpoint (with ra-tls) and cache the certificate

        Ok(Self {
            rpc_url,
            _cert: None,
        })
    }

    fn rpc_call<R: for<'de> Deserialize<'de>>(
        &self,
        method: &'static str,
        params: impl Serialize,
    ) -> Result<R> {
        let request = JsonRpcRequest {
            jsonrpc: "2.0",
            method,
            params,
            id: 1,
        };

        let response: JsonRpcResponse<R> = ureq::post(&self.rpc_url)
            .send_json(&request)
            .context("Failed to send RPC request")?
            .body_mut()
            .read_json()
            .context("Failed to parse RPC response")?;

        if let Some(error) = response.error {
            return Err(eyre!("RPC error {}: {}", error.code, error.message));
        }

        response.result.ok_or_else(|| eyre!("{:?}", response.error))
    }

    pub fn eth_balance_of(&self, account: Address) -> Result<U256> {
        let balance: String = self
            .rpc_call(
                "eth_getBalance",
                vec![format!("{:?}", account), "latest".to_string()],
            )
            .context("Failed to get ETH balance")?;

        U256::from_str_radix(balance.trim_start_matches("0x"), 16)
            .context("Failed to parse balance")
    }

    pub fn get_transaction_receipt(&self, hash: TxHash) -> Option<TransactionReceipt> {
        self.rpc_call("eth_getTransactionReceipt", vec![json!(hash)])
            .ok()
            .flatten()
            .inspect(|v| trace!("Transaction receipt: {v:#?}"))
    }

    /// Get nonce for an account
    pub fn get_transaction_count(&self, account: Address) -> Result<u64> {
        let nonce: String = self
            .rpc_call(
                "eth_getTransactionCount",
                vec![format!("{:?}", account), "latest".to_string()],
            )
            .context("Failed to get transaction count")?;

        u64::from_str_radix(nonce.trim_start_matches("0x"), 16).context("Failed to parse nonce")
    }

    /// Get current gas price
    pub fn gas_price(&self) -> Result<U256> {
        let gas_price: String = self
            .rpc_call("eth_gasPrice", json!([]))
            .context("Failed to get gas price")?;

        U256::from_str_radix(gas_price.trim_start_matches("0x"), 16)
            .context("Failed to parse gas price")
    }

    /// Estimate gas for a transaction
    pub fn estimate_gas(&self, from: Address, to: Address, data: Bytes) -> Result<U256> {
        self.rpc_call(
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
            .rpc_call::<Bytes>(
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
            .rpc_call("eth_chainId", json!([]))
            .context("Failed to get chain ID")?;
        u64::from_str_radix(chain_id.trim_start_matches("0x"), 16)
            .context("Failed to parse chain ID")
    }

    /// Get block by hash
    pub fn get_block_by_hash(&self, hash: TxHash) -> Result<Block> {
        self.rpc_call(
            "eth_getBlockByHash",
            vec![json!(format!("{:?}", hash)), json!(false)],
        )
        .context("Failed to get block by hash")
    }

    /// Get all receipts for a block
    pub fn get_block_receipts(&self, block_hash: TxHash) -> Result<Vec<TransactionReceipt>> {
        self.rpc_call("eth_getBlockReceipts", vec![format!("{:?}", block_hash)])
            .context("Failed to get block receipts")
    }
}
