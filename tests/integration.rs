//! Public API integration tests.
//!
//! These tests exercise the crate as an external caller would: constructing
//! state, building plans, and validating the public error/plan contracts.

use std::collections::HashMap;

use solana_account::Account;
use solana_program_pack::Pack;
use solana_pubkey::Pubkey;
use solana_signer::Signer;
use solana_token_toolkit::{
    detect_transfer_hooks, get_token_mint_and_transfer_fee, prepare_token_accounts, MintAndAta,
    MintIntent, TokenAccountIntent, TokenAccountState, TokenError, WrapSolStrategy,
};
use spl_token::state::{Account as SplTokenAccount, AccountState, Mint as SplMint};
use spl_token_interface::native_mint;

fn mint_account(token_program: Pubkey, decimals: u8) -> Account {
    let mint = SplMint {
        mint_authority: spl_token::solana_program::program_option::COption::None,
        supply: 0,
        decimals,
        is_initialized: true,
        freeze_authority: spl_token::solana_program::program_option::COption::None,
    };
    let mut data = vec![0u8; SplMint::LEN];
    SplMint::pack(mint, &mut data).unwrap();
    Account {
        lamports: 1_000_000,
        data,
        owner: token_program,
        executable: false,
        rent_epoch: 0,
    }
}

fn token_account_with_amount(amount: u64) -> Account {
    let token_acc = SplTokenAccount {
        mint: Pubkey::new_unique(),
        owner: Pubkey::new_unique(),
        amount,
        delegate: spl_token::solana_program::program_option::COption::None,
        state: AccountState::Initialized,
        is_native: spl_token::solana_program::program_option::COption::None,
        delegated_amount: 0,
        close_authority: spl_token::solana_program::program_option::COption::None,
    };
    let mut data = vec![0u8; SplTokenAccount::LEN];
    SplTokenAccount::pack(token_acc, &mut data).unwrap();
    Account {
        lamports: 2_039_280,
        data,
        owner: spl_token::ID,
        executable: false,
        rent_epoch: 0,
    }
}

fn state_with_entries(
    owner: Pubkey,
    entries: impl IntoIterator<Item = (Pubkey, MintAndAta)>,
) -> TokenAccountState {
    TokenAccountState {
        owner,
        mints: entries.into_iter().collect(),
    }
}

#[test]
fn classic_mint_parse_and_hook_detection_are_empty() {
    let mint = Pubkey::new_unique();
    let entry = MintAndAta {
        mint_account: mint_account(spl_token::ID, 6),
        ata_address: Pubkey::new_unique(),
        ata_account: None,
    };

    let parsed = get_token_mint_and_transfer_fee(mint, &entry.mint_account, 0).unwrap();
    assert_eq!(parsed.mint.decimals, 6);
    assert!(parsed.transfer_fee.is_none());

    let state = state_with_entries(Pubkey::new_unique(), [(mint, entry)]);
    assert!(detect_transfer_hooks(&state).is_empty());
}

#[test]
fn non_sol_ensure_ata_exists_builds_create_instruction_and_address() {
    let owner = Pubkey::new_unique();
    let mint = Pubkey::new_unique();
    let ata = Pubkey::new_unique();
    let state = state_with_entries(
        owner,
        [(
            mint,
            MintAndAta {
                mint_account: mint_account(spl_token::ID, 9),
                ata_address: ata,
                ata_account: None,
            },
        )],
    );
    let intent = TokenAccountIntent {
        mints: HashMap::from([(mint, MintIntent::EnsureAtaExists)]),
    };

    let plan = prepare_token_accounts(&state, &intent, WrapSolStrategy::Ata, 0).unwrap();

    assert_eq!(plan.create_instructions.len(), 1);
    assert!(plan.cleanup_instructions.is_empty());
    assert!(plan.additional_signers.is_empty());
    assert_eq!(plan.token_account_addresses[&mint], ata);
}

#[test]
fn wsol_ata_missing_with_balance_creates_wrap_and_cleanup_flow() {
    let owner = Pubkey::new_unique();
    let ata = Pubkey::new_unique();
    let state = state_with_entries(
        owner,
        [(
            native_mint::ID,
            MintAndAta {
                mint_account: mint_account(spl_token::ID, native_mint::DECIMALS),
                ata_address: ata,
                ata_account: None,
            },
        )],
    );
    let intent = TokenAccountIntent {
        mints: HashMap::from([(
            native_mint::ID,
            MintIntent::WithBalance {
                lamports: 1_000_000,
            },
        )]),
    };

    let plan = prepare_token_accounts(&state, &intent, WrapSolStrategy::Ata, 2_039_280).unwrap();

    assert_eq!(plan.create_instructions.len(), 3);
    assert_eq!(plan.cleanup_instructions.len(), 1);
    assert!(plan.additional_signers.is_empty());
    assert_eq!(plan.token_account_addresses[&native_mint::ID], ata);
}

#[test]
fn wsol_existing_ata_only_tops_up_delta_and_does_not_cleanup() {
    let owner = Pubkey::new_unique();
    let ata = Pubkey::new_unique();
    let state = state_with_entries(
        owner,
        [(
            native_mint::ID,
            MintAndAta {
                mint_account: mint_account(spl_token::ID, native_mint::DECIMALS),
                ata_address: ata,
                ata_account: Some(token_account_with_amount(400_000)),
            },
        )],
    );
    let intent = TokenAccountIntent {
        mints: HashMap::from([(
            native_mint::ID,
            MintIntent::WithBalance {
                lamports: 1_000_000,
            },
        )]),
    };

    let plan = prepare_token_accounts(&state, &intent, WrapSolStrategy::Ata, 2_039_280).unwrap();

    assert_eq!(plan.create_instructions.len(), 2);
    assert!(plan.cleanup_instructions.is_empty());
    assert!(plan.additional_signers.is_empty());
    assert_eq!(plan.token_account_addresses[&native_mint::ID], ata);
}

#[test]
fn keypair_strategy_returns_ephemeral_signer_and_account_address() {
    let owner = Pubkey::new_unique();
    let state = state_with_entries(
        owner,
        [(
            native_mint::ID,
            MintAndAta {
                mint_account: mint_account(spl_token::ID, native_mint::DECIMALS),
                ata_address: Pubkey::new_unique(),
                ata_account: None,
            },
        )],
    );
    let intent = TokenAccountIntent {
        mints: HashMap::from([(
            native_mint::ID,
            MintIntent::WithBalance {
                lamports: 1_000_000,
            },
        )]),
    };

    let plan =
        prepare_token_accounts(&state, &intent, WrapSolStrategy::Keypair, 2_039_280).unwrap();

    assert_eq!(plan.create_instructions.len(), 2);
    assert_eq!(plan.cleanup_instructions.len(), 1);
    assert_eq!(plan.additional_signers.len(), 1);
    assert_eq!(
        plan.token_account_addresses[&native_mint::ID],
        plan.additional_signers[0].pubkey()
    );
}

#[test]
fn invalid_intents_return_typed_errors() {
    let owner = Pubkey::new_unique();
    let mint = Pubkey::new_unique();
    let state = state_with_entries(
        owner,
        [(
            mint,
            MintAndAta {
                mint_account: mint_account(spl_token::ID, 9),
                ata_address: Pubkey::new_unique(),
                ata_account: None,
            },
        )],
    );

    let non_sol_with_balance = TokenAccountIntent {
        mints: HashMap::from([(mint, MintIntent::WithBalance { lamports: 1 })]),
    };
    let err =
        prepare_token_accounts(&state, &non_sol_with_balance, WrapSolStrategy::Ata, 0).unwrap_err();
    assert!(matches!(err, TokenError::WithBalanceNotSupported(m) if m == mint));

    let missing = Pubkey::new_unique();
    let missing_intent = TokenAccountIntent {
        mints: HashMap::from([(missing, MintIntent::EnsureAtaExists)]),
    };
    let err = prepare_token_accounts(&state, &missing_intent, WrapSolStrategy::Ata, 0).unwrap_err();
    assert!(matches!(err, TokenError::MintNotFound(m) if m == missing));
}
