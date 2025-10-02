use std::{
    collections::HashMap,
    io::{Read, Write},
    net::TcpStream,
};

use alloy_consensus::{SignableTransaction, TxLegacy};
use alloy_network::{eip2718::Encodable2718, TxSignerSync};
use alloy_primitives::utils::parse_ether;
use alloy_signer_local::PrivateKeySigner;
use alloy_sol_types::SolCall;
use color_eyre::{
    eyre::{bail, Context, ContextCompat},
    Result,
};
use nomad_types::{
    primitives::{Address, Bytes, TxHash, U256},
    Signal,
};
use serde::{Deserialize, Serialize};

use contracts::{Escrow, IERC20};
use tracing::{info, trace};

mod buildernet;
mod contracts;
mod geth;
mod proof;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct EthConfig {
    pub geth_rpc: String,
    pub builder_rpc: String,
    pub builder_atls: String,
    pub min_eth: f64,
}

impl EthConfig {
    pub fn read_from_stream(stream: &mut TcpStream) -> Result<Self> {
        // Read u32 length prefixed signal payload from the stream
        let mut len = [0u8; 4];
        stream.read_exact(&mut len)?;
        let len = u32::from_be_bytes(len) as usize;
        // Read payload
        let mut payload = vec![0u8; len];
        stream.read_exact(&mut payload)?;
        // Parse from json
        Ok(serde_json::from_slice(&payload)?)
    }
}

#[allow(unused)]
/// High level attested ethereum client
pub struct EthClient {
    keys: Vec<PrivateKeySigner>,
    accounts: Vec<Address>,
    bn: buildernet::BuildernetClient,
    geth: geth::GethClient,
    min_eth: U256,
    chain_id: u64,
    last_used_eoa_2: Option<usize>,
    nonces: HashMap<Address, u64>,
}

impl EthClient {
    pub fn new(keys: Vec<PrivateKeySigner>, config: EthConfig) -> Result<Self> {
        let accounts: Vec<Address> = keys.iter().map(|s| s.address()).collect();
        let geth = geth::GethClient::new(config.geth_rpc)?;
        let bn = buildernet::BuildernetClient::new(&config.builder_atls, config.builder_rpc)?;
        let min_eth = parse_ether(&config.min_eth.to_string())?;

        // Fetch chain_id from geth
        let chain_id = geth.get_chain_id()?;

        // Load nonces for all accounts
        let mut nonces = HashMap::new();
        for &account in &accounts {
            let nonce = geth.get_transaction_count(account)?;
            nonces.insert(account, nonce);
        }

        Ok(Self {
            keys,
            accounts,
            bn,
            geth,
            min_eth,
            chain_id,
            last_used_eoa_2: None,
            nonces,
        })
    }

    pub fn validate_signal(&self, signal: &Signal) -> Result<()> {
        if self.geth.escrow_is_bonded(signal.escrow_contract)? {
            bail!("Contract is already bonded");
        }
        if !self.geth.escrow_is_funded(signal.escrow_contract)? {
            bail!("Contract is not funded");
        }
        Ok(())
    }

    /// Get accounts above minimum eth balance, or return error if not at least 2
    fn get_active_accounts(&self) -> Result<Vec<usize>> {
        let mut active = Vec::new();
        let mut inactive = Vec::new();
        for (i, address) in self.accounts.iter().cloned().enumerate() {
            if self.geth.eth_balance_of(address)? >= self.min_eth {
                active.push(i);
            } else {
                inactive.push(i);
            }
        }
        if active.len() < 2 {
            bail!("not enough eth");
        }
        Ok(active)
    }

    /// Get contract balances
    fn token_balances(&self, accounts: &[usize], contract: Address) -> Result<Vec<U256>> {
        accounts
            .iter()
            .map(|a| self.geth.erc20_balance_of(contract, self.accounts[*a]))
            .collect()
    }

    /// Select ideal accounts for EOA 1 and 2
    pub fn select_accounts(&mut self, signal: &Signal) -> Result<[usize; 2]> {
        let accounts = self.get_active_accounts()?;
        let mut balances = self
            .token_balances(&accounts, signal.token_contract)?
            .into_iter()
            .zip(accounts)
            .collect::<Vec<_>>();

        // Compute minimum bond amount
        let bond_amount = signal
            .reward_amount
            .checked_mul(U256::from(52))
            .unwrap()
            .checked_div(U256::from(100))
            .unwrap();

        // Get the last used EOA 2 account for this token, if any
        let last_used_eoa_2 = self.last_used_eoa_2;

        // find eoa 1; needs enough for bond amount.
        // should have the least amount of funds for redistribution
        balances.sort();
        let eoa_1 = balances
            .iter()
            .find(|(bal, _)| bal >= &bond_amount)
            .context("failed to select eoa 1")?
            .1;

        // find eoa 2; needs enough for escrow.
        // should have the most amount of funds for redistribution
        // but avoid reusing the last used EOA 2 account
        balances.reverse();
        let eoa_2 = balances
            .iter()
            .find(|(bal, i)| {
                i != &eoa_1 && bal >= &signal.transfer_amount && Some(*i) != last_used_eoa_2
            })
            .or_else(|| {
                // If we can't find an account that wasn't last used as EOA 2, fall back to any valid account
                balances
                    .iter()
                    .find(|(bal, i)| i != &eoa_1 && bal >= &signal.transfer_amount)
            })
            .context("failed to find eoa 2")?
            .1;

        // Track this EOA 2 account as the last used for this token
        self.last_used_eoa_2 = Some(eoa_2);

        Ok([eoa_1, eoa_2])
    }

    /// Helper to build, sign, and send a transaction for a contract call
    fn send_transaction(
        &mut self,
        stream: &mut TcpStream,
        eoa_index: usize,
        to: Address,
        call: impl SolCall,
    ) -> Result<TxHash> {
        let signer = &self.keys[eoa_index];
        let from = signer.address();

        // Get and increment stored nonce
        let nonce = self
            .nonces
            .get_mut(&from)
            .context("Account nonce not found")?;
        let current_nonce = *nonce;
        *nonce += 1;

        let gas_price = self.geth.gas_price()?;

        // Encode call data
        let data = Bytes::from(call.abi_encode());

        // Estimate gas
        let gas_limit = self
            .geth
            .estimate_gas(from, to, data.clone())?
            .try_into()
            .context("gas estimation greater than u46::MAX")?;

        // Build legacy transaction
        let mut tx = TxLegacy {
            chain_id: Some(self.chain_id),
            nonce: current_nonce,
            gas_price: gas_price.to::<u128>(),
            gas_limit,
            to: to.into(),
            value: U256::ZERO,
            input: data,
        };

        // Sign transaction
        let signature = signer.sign_transaction_sync(&mut tx)?;
        let signed = tx.into_signed(signature);

        // Send via buildernet
        let tx_hash = self.bn.send_raw_transaction(signed.encoded_2718().into())?;

        // Poll for the transaction receipt
        let max_attempts = 60; // ~1 minute with 1s intervals
        let mut receipt = None;
        for _ in 0..max_attempts {
            trace!("Polling transaction receipt");
            if let Some(r) = self.geth.get_transaction_receipt(tx_hash) {
                receipt = Some(r);
                break;
            }
            // request timeout from userspace and wait for response
            stream.write_all(&u32::MAX.to_be_bytes())?;
            stream.read_exact(&mut [0])?;
        }

        let receipt = receipt.context("Transaction receipt not found after polling")?;
        trace!("Received transaction receipt for {tx_hash}");
        if !receipt.status() {
            bail!("Transaction failed");
        }

        Ok(tx_hash)
    }

    /// Bond to a signal with a given eoa
    pub fn bond(
        &mut self,
        stream: &mut TcpStream,
        eoa_1: usize,
        signal: &Signal,
    ) -> Result<[TxHash; 2]> {
        // Compute minimum bond amount
        let bond_amount = signal
            .reward_amount
            .checked_mul(U256::from(52))
            .unwrap()
            .checked_div(U256::from(100))
            .unwrap();

        info!("Approving tokens");

        // Approve bond amount for escrow contract, on the token contract
        let approve_tx = self
            .send_transaction(
                stream,
                eoa_1,
                signal.token_contract,
                IERC20::approveCall {
                    spender: signal.escrow_contract,
                    value: bond_amount,
                },
            )
            .context("failed to approve tokens")?;

        info!("Bonding escrow");

        // Send bond call to escrow contract
        let bond_tx = self
            .send_transaction(
                stream,
                eoa_1,
                signal.escrow_contract,
                Escrow::bondCall(bond_amount),
            )
            .context("failed to bond to contract")?;

        Ok([approve_tx, bond_tx])
    }

    /// Execute the transfer for a signal using a given eoa
    pub fn transfer(
        &mut self,
        stream: &mut TcpStream,
        eoa_2: usize,
        signal: &Signal,
    ) -> Result<TxHash> {
        self.send_transaction(
            stream,
            eoa_2,
            signal.token_contract,
            IERC20::transferCall {
                to: signal.recipient,
                value: signal.transfer_amount,
            },
        )
        .context("failed to transfer tokens")
    }

    /// Collect a reward for a signal with a given eoa
    pub fn collect(
        &mut self,
        stream: &mut TcpStream,
        eoa_1: usize,
        signal: &Signal,
        transfer_tx: TxHash,
    ) -> Result<TxHash> {
        // Generate proof for the transfer transaction
        let proof = self.generate_proof(transfer_tx, signal.recipient, signal.transfer_amount)?;
        let receipt = self.geth.get_transaction_receipt(transfer_tx).unwrap();
        let block_number = receipt
            .block_number
            .context("Block number not found in receipt")?;
        trace!("block number {block_number}");

        // Call collect on the escrow contract
        self.send_transaction(
            stream,
            eoa_1,
            signal.escrow_contract,
            Escrow::collectCall {
                proof,
                targetBlockNumber: U256::from(block_number),
            },
        )
    }

    /// Try to swap for some eth, ensuring we retain a minimum amount of tokens
    pub fn _try_swap(
        &self,
        _eoa: usize,
        _token: Address,
        _target_eth: U256,
        _min_tokens: U256,
    ) -> Result<()> {
        todo!()
    }

    /// Create a mirage signal redistributing funds from an EOA to a destination address.
    /// Used for node runner withdraws and account balance recovery.
    pub fn _redistribute(
        &self,
        _source: usize,
        _target: Address,
        _token: Address,
        _amount: U256,
    ) -> Result<()> {
        todo!()
    }

    pub fn chain_id(&self) -> u64 {
        self.chain_id
    }
}
