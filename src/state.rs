//! Tier 1 — on-chain state queries for an owner's token accounts.

use std::collections::HashMap;

use solana_account::Account;
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_pubkey::Pubkey;
use spl_associated_token_account_interface::address::get_associated_token_address_with_program_id;
use spl_token_interface::native_mint;

use crate::TokenError;

/// On-chain state for an owner's token accounts across a set of mints.
#[derive(Debug, Clone)]
pub struct TokenAccountState {
    /// The owner whose ATAs were queried.
    pub owner: Pubkey,
    /// Map of mint pubkey to per-mint state.
    pub mints: HashMap<Pubkey, MintAndAta>,
}

impl TokenAccountState {
    /// Build an empty state (no mints) for the given owner.
    #[must_use]
    pub fn empty(owner: Pubkey) -> Self {
        Self {
            owner,
            mints: HashMap::new(),
        }
    }
}

/// Per-mint state: the mint account itself, the ATA address, and the
/// ATA account if it currently exists on chain.
#[derive(Debug, Clone)]
pub struct MintAndAta {
    /// The raw mint account (use `spl-token-2022-interface` to unpack).
    pub mint_account: Account,
    /// Derived ATA address for `(owner, mint, mint_account.owner)`.
    pub ata_address: Pubkey,
    /// `Some` if the ATA exists on chain, `None` if it needs creation.
    pub ata_account: Option<Account>,
}

/// Fetch the on-chain state for an owner across a set of mints. Performs
/// two RPC `get_multiple_accounts` calls (mints, then ATAs).
///
/// Returns `Err(TokenError::MintNotFound(mint))` if any requested mint
/// account does not exist on chain.
///
/// # Example
///
/// ```no_run
/// # use solana_token_toolkit::{fetch_token_account_state, TokenError};
/// # use solana_client::nonblocking::rpc_client::RpcClient;
/// # use solana_pubkey::Pubkey;
/// # async fn run() -> Result<(), TokenError> {
/// let rpc = RpcClient::new("http://localhost:8899".to_string());
/// let owner = Pubkey::new_unique();
/// let mints = vec![Pubkey::new_unique()];
/// let state = fetch_token_account_state(&rpc, owner, &mints).await?;
/// # let _ = state;
/// # Ok(())
/// # }
/// ```
pub async fn fetch_token_account_state(
    rpc: &RpcClient,
    owner: Pubkey,
    mints: &[Pubkey],
) -> Result<TokenAccountState, TokenError> {
    if mints.is_empty() {
        return Ok(TokenAccountState::empty(owner));
    }

    let mint_account_opts = rpc.get_multiple_accounts(mints).await?;
    let mut mint_accounts = Vec::with_capacity(mints.len());
    for (i, opt) in mint_account_opts.into_iter().enumerate() {
        match (mints[i] == native_mint::ID, opt) {
            (_, Some(account)) => mint_accounts.push(account),
            (true, None) => mint_accounts.push(native_mint_account()),
            (false, None) => return Err(TokenError::MintNotFound(mints[i])),
        }
    }

    assemble_token_account_state(rpc, owner, mints, &mint_accounts).await
}

fn native_mint_account() -> Account {
    use solana_program_pack::Pack;
    use spl_token::state::Mint;

    let mint = Mint {
        mint_authority: spl_token::solana_program::program_option::COption::None,
        supply: 0,
        decimals: native_mint::DECIMALS,
        is_initialized: true,
        freeze_authority: spl_token::solana_program::program_option::COption::None,
    };
    let mut data = vec![0u8; Mint::LEN];
    Mint::pack(mint, &mut data).expect("native mint account serialization is infallible");

    Account {
        lamports: 0,
        data,
        owner: spl_token::ID,
        executable: false,
        rent_epoch: 0,
    }
}

/// Assemble token account state from caller-provided mint accounts. Performs
/// one RPC `get_multiple_accounts` call (ATAs only).
///
/// `mints` and `mint_accounts` must be aligned by index.
///
/// # Example
///
/// ```no_run
/// # use solana_token_toolkit::{assemble_token_account_state, TokenError};
/// # use solana_client::nonblocking::rpc_client::RpcClient;
/// # use solana_pubkey::Pubkey;
/// # async fn run() -> Result<(), TokenError> {
/// let rpc = RpcClient::new("http://localhost:8899".to_string());
/// let owner = Pubkey::new_unique();
/// let mints = vec![Pubkey::new_unique()];
/// let mint_accounts = rpc.get_multiple_accounts(&mints).await?
///     .into_iter()
///     .collect::<Option<Vec<_>>>()
///     .ok_or(TokenError::MintNotFound(mints[0]))?;
/// let state = assemble_token_account_state(&rpc, owner, &mints, &mint_accounts).await?;
/// # let _ = state;
/// # Ok(())
/// # }
/// ```
pub async fn assemble_token_account_state(
    rpc: &RpcClient,
    owner: Pubkey,
    mints: &[Pubkey],
    mint_accounts: &[Account],
) -> Result<TokenAccountState, TokenError> {
    if mints.len() != mint_accounts.len() {
        return Err(TokenError::LengthMismatch {
            mints: mints.len(),
            mint_accounts: mint_accounts.len(),
        });
    }
    if mints.is_empty() {
        return Ok(TokenAccountState::empty(owner));
    }

    let ata_addresses: Vec<Pubkey> = mints
        .iter()
        .zip(mint_accounts.iter())
        .map(|(mint, acc)| get_associated_token_address_with_program_id(&owner, mint, &acc.owner))
        .collect();

    let ata_account_opts = rpc.get_multiple_accounts(&ata_addresses).await?;

    let mints_map = mints
        .iter()
        .zip(mint_accounts.iter())
        .zip(ata_addresses.iter())
        .zip(ata_account_opts)
        .map(|(((mint, mint_account), ata_address), ata_account)| {
            (
                *mint,
                MintAndAta {
                    mint_account: mint_account.clone(),
                    ata_address: *ata_address,
                    ata_account,
                },
            )
        })
        .collect();

    Ok(TokenAccountState {
        owner,
        mints: mints_map,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn empty_mints_returns_empty_state_no_rpc() {
        let rpc = RpcClient::new_mock("succeeds".to_string());
        let owner = Pubkey::new_unique();
        let state = assemble_token_account_state(&rpc, owner, &[], &[])
            .await
            .unwrap();
        assert_eq!(state.owner, owner);
        assert!(state.mints.is_empty());
    }

    #[tokio::test]
    async fn length_mismatch_returns_error_no_rpc() {
        let rpc = RpcClient::new_mock("succeeds".to_string());
        let owner = Pubkey::new_unique();
        let mints = vec![Pubkey::new_unique(), Pubkey::new_unique()];
        let mint_accounts = vec![Account {
            lamports: 0,
            data: vec![],
            owner: spl_token::ID,
            executable: false,
            rent_epoch: 0,
        }];
        let err = assemble_token_account_state(&rpc, owner, &mints, &mint_accounts)
            .await
            .unwrap_err();
        match err {
            TokenError::LengthMismatch {
                mints: m,
                mint_accounts: ma,
            } => {
                assert_eq!(m, 2);
                assert_eq!(ma, 1);
            }
            _ => panic!("expected LengthMismatch, got {err:?}"),
        }
    }

    #[test]
    fn empty_state_constructor() {
        let owner = Pubkey::new_unique();
        let state = TokenAccountState::empty(owner);
        assert_eq!(state.owner, owner);
        assert!(state.mints.is_empty());
    }

    #[tokio::test]
    async fn fetch_empty_mints_returns_empty_state_no_rpc() {
        let rpc = RpcClient::new_mock("succeeds".to_string());
        let owner = Pubkey::new_unique();
        let state = fetch_token_account_state(&rpc, owner, &[]).await.unwrap();
        assert_eq!(state.owner, owner);
        assert!(state.mints.is_empty());
    }

    #[test]
    fn native_mint_account_is_valid_classic_mint_owned_by_spl_token() {
        use solana_program_pack::Pack;
        use spl_token::state::Mint;

        let account = native_mint_account();
        assert_eq!(account.owner, spl_token::ID);
        let mint = Mint::unpack(&account.data).unwrap();
        assert!(mint.is_initialized);
        assert_eq!(mint.decimals, native_mint::DECIMALS);
    }
}
