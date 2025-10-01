//! # GETH RA-TLS Client
//!
//! Connects directly to a GETH Enclave and provides RPC client implementation for our EOAs
//! - Obfuscated contract write calls
//! - Balance checks (ETH, ERC20)
//! - Hidden transaction polling (buildernet wont publish until its in a block)
//! - Transaction Receipts

use alloy_sol_types::SolCall;
use eyre::{Context, Result};
use nomad_types::primitives::{Address, Bytes, TxHash, U256};
use serde::{Deserialize, Serialize};
use serde_json::json;

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
    cert: Option<String>,
}

impl GethClient {
    pub fn new(rpc_url: String) -> eyre::Result<Self> {
        // TODO: connect to the rpc endpoint (with ra-tls) and cache the certificate

        Ok(Self {
            rpc_url,
            cert: None,
        })
    }

    fn rpc_call<P: Serialize, R: for<'de> Deserialize<'de>>(
        &self,
        method: &'static str,
        params: P,
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
            return Err(eyre::eyre!("RPC error {}: {}", error.code, error.message));
        }

        response
            .result
            .ok_or_else(|| eyre::eyre!("RPC response missing result"))
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

    pub fn get_block(&self) -> Result<u64> {
        let block_number: String = self
            .rpc_call("eth_blockNumber", json!([]))
            .context("Failed to get block number")?;

        u64::from_str_radix(block_number.trim_start_matches("0x"), 16)
            .context("Failed to parse block number")
    }

    pub fn get_transaction(&self, hash: TxHash) -> Result<Option<serde_json::Value>> {
        self.rpc_call("eth_getTransactionByHash", vec![format!("{:?}", hash)])
            .context("Failed to get transaction")
    }

    pub fn get_transaction_receipt(&self, hash: TxHash) -> Result<Option<serde_json::Value>> {
        self.rpc_call("eth_getTransactionReceipt", vec![format!("{:?}", hash)])
            .context("Failed to get transaction receipt")
    }

    /// Send a raw signed transaction
    pub fn send_raw_transaction(&self, signed_tx: Bytes) -> Result<TxHash> {
        let tx_hash: String = self
            .rpc_call(
                "eth_sendRawTransaction",
                vec![format!("0x{}", hex::encode(signed_tx))],
            )
            .context("Failed to send raw transaction")?;

        Ok(TxHash::from_slice(&hex::decode(
            tx_hash.trim_start_matches("0x"),
        )?))
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
    pub fn estimate_gas(&self, from: Address, to: Address, data: Bytes) -> Result<u64> {
        let gas: String = self
            .rpc_call(
                "eth_estimateGas",
                vec![json!({
                    "from": format!("{:?}", from),
                    "to": format!("{:?}", to),
                    "data": &data,
                })],
            )
            .context("Failed to estimate gas")?;

        u64::from_str_radix(gas.trim_start_matches("0x"), 16).context("Failed to parse gas")
    }


    /// Call eth_call for contract view functions
    pub fn eth_call(&self, to: Address, data: impl SolCall) -> Result<Bytes> {
        let result: String = self
            .rpc_call(
                "eth_call",
                vec![
                    json!({
                        "to": format!("{:?}", to),
                        "data": &Bytes::from(data.abi_encode()),
                    }),
                    json!("latest"),
                ],
            )
            .context("Failed to call eth_call")?;

        Ok(Bytes::from(hex::decode(result.trim_start_matches("0x"))?))
    }

    /// Get erc20 balance for contract
    pub fn erc20_balance_of(&self, token: Address, account: Address) -> Result<U256> {
        let result = self.eth_call(token, IERC20::balanceOfCall(account))?;
        Ok(U256::from_be_slice(&result))
    }

    /// Get erc20 decimals
    pub fn erc20_decimals(&self, token: Address) -> Result<u8> {
        let result = self.eth_call(token, IERC20::decimalsCall {})?;
        // Decode uint8 from 32 bytes (last byte contains the value)
        Ok(result[31])
    }

    /// Check if an escrow is bonded
    pub fn escrow_is_bonded(&self, escrow: Address) -> Result<bool> {
        let result = self.eth_call(escrow, Escrow::is_bondedCall {})?;
        // Decode boolean result (32 bytes, last byte is 0 or 1)
        Ok(result.len() >= 32 && result[31] != 0)
    }

    /// Get chain ID
    pub fn get_chain_id(&self) -> Result<u64> {
        let chain_id: String = self
            .rpc_call("eth_chainId", json!([]))
            .context("Failed to get chain ID")?;

        u64::from_str_radix(chain_id.trim_start_matches("0x"), 16).context("Failed to parse chain ID")
    }
}
