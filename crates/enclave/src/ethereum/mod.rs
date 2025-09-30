#![allow(unused)]

use eyre::{bail, ContextCompat, Result};
use nomad_types::{
    primitives::{Address, TxHash, U256},
    Signal,
};

mod buildernet;
mod geth;

/// High level attested ethereum client
pub struct EthClient {
    keys: Vec<[u8; 32]>,
    accounts: Vec<Address>,
    bn: buildernet::BuildernetClient,
    geth: geth::GethClient,
    min_eth: U256,
    last_used_eoa_2: Option<usize>,
}

impl EthClient {
    pub fn new(
        keys: Vec<[u8; 32]>,
        bn_atls_url: &str,
        bn_rpc_url: String,
        geth_url: String,
        min_eth: U256,
    ) -> Result<Self> {
        Ok(Self {
            keys,
            // TODO: derive pks
            accounts: vec![],
            bn: buildernet::BuildernetClient::new(bn_atls_url, bn_rpc_url)?,
            geth: geth::GethClient::new(geth_url)?,
            min_eth,
            last_used_eoa_2: None,
        })
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
        self.accounts
            .iter()
            .map(|a| self.geth.erc20_balance_of(*a))
            .collect()
    }

    /// Select ideal accounts for EOA 1 and 2
    pub fn select_accounts(&mut self, signal: &Signal) -> Result<[usize; 2]> {
        let accounts = self.get_active_accounts()?;
        let mut balances = self
            .token_balances(&accounts, signal.token_contract)?
            .into_iter()
            .enumerate()
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
            .find(|(_, bal)| bal >= &bond_amount)
            .context("failed to select eoa 1")?
            .0;

        // find eoa 2; needs enough for escrow.
        // should have the most amount of funds for redistribution
        // but avoid reusing the last used EOA 2 account
        balances.reverse();
        let eoa_2 = balances
            .iter()
            .find(|(i, bal)| {
                i != &eoa_1 && bal >= &signal.transfer_amount && Some(*i) != last_used_eoa_2
            })
            .or_else(|| {
                // If we can't find an account that wasn't last used as EOA 2, fall back to any valid account
                balances
                    .iter()
                    .find(|(i, bal)| i != &eoa_1 && bal >= &signal.transfer_amount)
            })
            .context("failed to find eoa 2")?
            .0;

        // Track this EOA 2 account as the last used for this token
        self.last_used_eoa_2 = Some(eoa_2);

        Ok([eoa_1, eoa_2])
    }

    /// Bond to a signal with a given eoa
    pub fn bond(&self, eoa_1: usize, signal: &Signal) -> Result<[TxHash; 2]> {
        todo!()
    }

    /// Execute the transfer for a signal using a given eoa
    pub fn transfer(&self, eoa_2: usize, signal: &Signal) -> Result<TxHash> {
        todo!()
    }

    /// Collect a reward for a signal with a given eoa
    pub fn collect(&self, eoa_1: usize, signal: &Signal, transfer_tx: TxHash) -> Result<TxHash> {
        todo!()
    }

    /// Try to swap for some eth, ensuring we retain a minimum amount of tokens
    pub fn try_swap(
        &self,
        eoa: usize,
        token: Address,
        target_eth: U256,
        min_tokens: U256,
    ) -> Result<()> {
        todo!()
    }

    /// Create a mirage signal redistributing funds from an EOA to a destination address.
    /// Used for node runner withdraws and account balance recovery.
    pub fn redistribute(
        &self,
        _source: usize,
        _target: Address,
        _token: Address,
        _amount: U256,
    ) -> Result<()> {
        todo!()
    }
}
