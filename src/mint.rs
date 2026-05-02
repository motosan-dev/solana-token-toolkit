//! Tier 2 — Token-2022 mint extension utilities.

use std::collections::HashMap;

use solana_account::Account;
use solana_pubkey::Pubkey;
use spl_token_2022_interface::{
    extension::{
        transfer_fee::TransferFeeConfig, transfer_hook::TransferHook, BaseStateWithExtensions,
        StateWithExtensions,
    },
    state::Mint,
};

use crate::{state::TokenAccountState, TokenError};

/// Epoch-aware transfer fee summary for a Token-2022 mint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TransferFee {
    /// Fee in basis points (1 bps = 0.01%).
    pub fee_bps: u16,
    /// Maximum fee in raw token units regardless of transfer size.
    pub max_fee: u64,
}

/// A parsed mint plus its current transfer fee (if any).
#[derive(Debug, Clone)]
pub struct TokenMintWithFee {
    /// The unpacked mint base state.
    pub mint: Mint,
    /// `Some` if the mint has the TransferFee extension, `None` otherwise.
    pub transfer_fee: Option<TransferFee>,
}

/// Parse a mint account and resolve its transfer fee for the given epoch.
///
/// # Example
///
/// ```no_run
/// # use solana_token_toolkit::{get_token_mint_and_transfer_fee, TokenError};
/// # use solana_account::Account;
/// # use solana_pubkey::Pubkey;
/// # fn run(mint_account: Account) -> Result<(), TokenError> {
/// let parsed = get_token_mint_and_transfer_fee(Pubkey::new_unique(), &mint_account, 100)?;
/// # let _ = parsed;
/// # Ok(())
/// # }
/// ```
pub fn get_token_mint_and_transfer_fee(
    mint_pubkey: Pubkey,
    mint_account: &Account,
    epoch: u64,
) -> Result<TokenMintWithFee, TokenError> {
    let unpacked = StateWithExtensions::<Mint>::unpack(&mint_account.data).map_err(|e| {
        TokenError::MintDecodeFailed {
            mint: mint_pubkey,
            reason: format!("StateWithExtensions::unpack: {e}"),
        }
    })?;

    let transfer_fee = unpacked
        .get_extension::<TransferFeeConfig>()
        .ok()
        .map(|cfg| {
            let fee = cfg.get_epoch_fee(epoch);
            TransferFee {
                fee_bps: fee.transfer_fee_basis_points.into(),
                max_fee: fee.maximum_fee.into(),
            }
        });

    Ok(TokenMintWithFee {
        mint: unpacked.base,
        transfer_fee,
    })
}

/// Information about a transfer hook attached to a Token-2022 mint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct TransferHookInfo {
    /// The program ID that the mint's transfer hook calls into.
    pub hook_program_id: Pubkey,
}

/// Detect transfer hooks on every mint in the given state.
///
/// # Example
///
/// ```no_run
/// # use solana_token_toolkit::{detect_transfer_hooks, TokenAccountState};
/// # use solana_pubkey::Pubkey;
/// let hooks = detect_transfer_hooks(&TokenAccountState::empty(Pubkey::new_unique()));
/// assert!(hooks.is_empty());
/// ```
#[must_use]
pub fn detect_transfer_hooks(state: &TokenAccountState) -> HashMap<Pubkey, TransferHookInfo> {
    let mut out = HashMap::new();
    for (mint_pubkey, entry) in &state.mints {
        if let Ok(unpacked) = StateWithExtensions::<Mint>::unpack(&entry.mint_account.data) {
            if let Ok(hook) = unpacked.get_extension::<TransferHook>() {
                if let Some(program_id) = Option::<Pubkey>::from(hook.program_id) {
                    out.insert(
                        *mint_pubkey,
                        TransferHookInfo {
                            hook_program_id: program_id,
                        },
                    );
                }
            }
        }
    }
    out
}

/// Strict-mode policy: return the lowest-pubkey transfer-hook detection as an error.
///
/// # Example
///
/// ```no_run
/// # use solana_token_toolkit::{reject_transfer_hook_mints, TokenAccountState};
/// # use solana_pubkey::Pubkey;
/// reject_transfer_hook_mints(&TokenAccountState::empty(Pubkey::new_unique())).unwrap();
/// ```
pub fn reject_transfer_hook_mints(state: &TokenAccountState) -> Result<(), TokenError> {
    let hooks = detect_transfer_hooks(state);
    let mut sorted: Vec<(Pubkey, TransferHookInfo)> = hooks.into_iter().collect();
    sorted.sort_by_key(|(pubkey, _)| *pubkey);
    if let Some((mint, info)) = sorted.into_iter().next() {
        return Err(TokenError::TransferHookDetected {
            mint,
            program: info.hook_program_id,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use solana_program_pack::Pack;
    use spl_token::state::Mint as ClassicMint;

    use super::*;
    use crate::state::{MintAndAta, TokenAccountState};

    fn make_classic_mint_account(decimals: u8) -> Account {
        let mint = ClassicMint {
            mint_authority: spl_token::solana_program::program_option::COption::None,
            supply: 0,
            decimals,
            is_initialized: true,
            freeze_authority: spl_token::solana_program::program_option::COption::None,
        };
        let mut data = vec![0u8; ClassicMint::LEN];
        ClassicMint::pack(mint, &mut data).unwrap();
        Account {
            lamports: 1_000_000,
            data,
            owner: spl_token::ID,
            executable: false,
            rent_epoch: 0,
        }
    }

    #[test]
    fn classic_mint_decodes_with_no_transfer_fee() {
        let parsed = get_token_mint_and_transfer_fee(
            Pubkey::new_unique(),
            &make_classic_mint_account(9),
            100,
        )
        .unwrap();
        assert_eq!(parsed.mint.decimals, 9);
        assert!(parsed.transfer_fee.is_none());
    }

    #[test]
    fn malformed_mint_data_returns_decode_error() {
        let mint_pubkey = Pubkey::new_unique();
        let account = Account {
            lamports: 0,
            data: vec![0xFF; 32],
            owner: spl_token::ID,
            executable: false,
            rent_epoch: 0,
        };
        let err = get_token_mint_and_transfer_fee(mint_pubkey, &account, 100).unwrap_err();
        assert!(matches!(err, TokenError::MintDecodeFailed { mint, .. } if mint == mint_pubkey));
    }

    fn state_with_one_mint(mint: Pubkey, account: Account) -> TokenAccountState {
        let mut mints = HashMap::new();
        mints.insert(
            mint,
            MintAndAta {
                mint_account: account,
                ata_address: Pubkey::new_unique(),
                ata_account: None,
            },
        );
        TokenAccountState {
            owner: Pubkey::new_unique(),
            mints,
        }
    }

    #[test]
    fn detect_transfer_hooks_empty_for_classic_mint() {
        let state = state_with_one_mint(Pubkey::new_unique(), make_classic_mint_account(9));
        assert!(detect_transfer_hooks(&state).is_empty());
    }
}
