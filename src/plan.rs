//! Tier 3 — token-account preparation plan generation.

use std::collections::HashMap;

use solana_account::Account;
use solana_instruction::Instruction;
use solana_keypair::Keypair;
use solana_pubkey::Pubkey;
use solana_signer::Signer;
use solana_system_interface::instruction as system_instruction;
use spl_associated_token_account_interface::instruction::{
    create_associated_token_account, create_associated_token_account_idempotent,
};
use spl_token_2022_interface::instruction::{close_account, initialize_account3, sync_native};
use spl_token_interface::native_mint;

use crate::{
    state::{MintAndAta, TokenAccountState},
    TokenError,
};

/// Build the ATA create instruction matching the configured mode.
fn build_create_ata_instruction(
    mode: AtaCreateMode,
    payer: &Pubkey,
    owner: &Pubkey,
    mint: &Pubkey,
    token_program: &Pubkey,
) -> Instruction {
    match mode {
        AtaCreateMode::Idempotent => {
            create_associated_token_account_idempotent(payer, owner, mint, token_program)
        }
        AtaCreateMode::Legacy => create_associated_token_account(payer, owner, mint, token_program),
    }
}

/// What the caller wants for each mint.
#[derive(Debug, Clone)]
pub struct TokenAccountIntent {
    /// Map of mint pubkey to intent.
    pub mints: HashMap<Pubkey, MintIntent>,
}

/// Per-mint intent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MintIntent {
    /// Just ensure the ATA exists. No balance preparation.
    EnsureAtaExists,
    /// SOL/wSOL only: ensure the wSOL account holds at least `lamports`.
    WithBalance {
        /// Required lamport balance in the wSOL account.
        lamports: u64,
    },
    /// Non-SOL mints only: validate at plan time that the existing token
    /// account already holds at least `amount`.
    RequireTokenBalance {
        /// Minimum required balance in raw token units.
        amount: u64,
    },
}

/// How to wrap native SOL when needed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WrapSolStrategy {
    /// Use the persistent wSOL ATA.
    Ata,
    /// Create an ephemeral keypair-based wSOL account.
    Keypair,
    /// Do not wrap. Incoherent with `MintIntent::WithBalance` for native SOL.
    None,
}

/// Whether `prepare_token_accounts` emits idempotent or non-idempotent ATA
/// creation instructions.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum AtaCreateMode {
    /// Idempotent ATA creation (default).
    #[default]
    Idempotent,
    /// Non-idempotent ATA creation (legacy / byte-compat).
    Legacy,
}

/// Configuration for `prepare_token_accounts`.
///
/// **No `Default` impl** — `rent_exempt_lamports` has no safe default. Use
/// `with_rent(...)` to build with sensible defaults for other fields.
#[derive(Debug, Clone)]
pub struct TokenAccountPlanConfig {
    /// How to handle native SOL wrapping.
    pub wsol_strategy: WrapSolStrategy,
    /// Whether ATA creation instructions are idempotent or legacy.
    pub ata_create_mode: AtaCreateMode,
    /// Rent-exempt lamports for a 165-byte SPL Token account.
    pub rent_exempt_lamports: u64,
}

impl TokenAccountPlanConfig {
    /// Construct a config with `WrapSolStrategy::Ata` and
    /// `AtaCreateMode::Idempotent` defaults.
    #[must_use]
    pub fn with_rent(rent_exempt_lamports: u64) -> Self {
        Self {
            wsol_strategy: WrapSolStrategy::Ata,
            ata_create_mode: AtaCreateMode::Idempotent,
            rent_exempt_lamports,
        }
    }
}

/// Output of `prepare_token_accounts`.
#[derive(Debug)]
pub struct TokenAccountPlan {
    /// Instructions to execute before the main transaction body.
    pub create_instructions: Vec<Instruction>,
    /// Instructions to execute after the main transaction body.
    pub cleanup_instructions: Vec<Instruction>,
    /// Ephemeral keypairs for `WrapSolStrategy::Keypair`.
    pub additional_signers: Vec<Keypair>,
    /// Mint pubkey to actual token account address.
    pub token_account_addresses: HashMap<Pubkey, Pubkey>,
}

/// Build the token-account preparation plan for a transaction.
///
/// # Example
///
/// ```no_run
/// # use std::collections::HashMap;
/// # use solana_token_toolkit::*;
/// # use solana_pubkey::Pubkey;
/// let owner = Pubkey::new_unique();
/// let state = TokenAccountState::empty(owner);
/// let intent = TokenAccountIntent { mints: HashMap::new() };
/// let plan = prepare_token_accounts(
///     &state,
///     &intent,
///     TokenAccountPlanConfig::with_rent(0),
/// ).unwrap();
/// assert!(plan.create_instructions.is_empty());
/// ```
pub fn prepare_token_accounts(
    state: &TokenAccountState,
    intent: &TokenAccountIntent,
    config: TokenAccountPlanConfig,
) -> Result<TokenAccountPlan, TokenError> {
    let mut plan = TokenAccountPlan {
        create_instructions: Vec::new(),
        cleanup_instructions: Vec::new(),
        additional_signers: Vec::new(),
        token_account_addresses: HashMap::new(),
    };

    let mut sorted_intent: Vec<(&Pubkey, &MintIntent)> = intent.mints.iter().collect();
    sorted_intent.sort_by_key(|(pubkey, _)| **pubkey);

    for (mint_pubkey, mint_intent) in sorted_intent {
        let entry = state
            .mints
            .get(mint_pubkey)
            .ok_or(TokenError::MintNotFound(*mint_pubkey))?;
        let is_native_sol = *mint_pubkey == native_mint::ID;

        match (is_native_sol, mint_intent) {
            (true, MintIntent::WithBalance { lamports }) => handle_wrap_sol(
                state.owner,
                entry,
                *lamports,
                config.wsol_strategy,
                config.rent_exempt_lamports,
                config.ata_create_mode,
                &mut plan,
            )?,
            (true, MintIntent::EnsureAtaExists) | (false, MintIntent::EnsureAtaExists) => {
                ensure_ata_exists(
                    state.owner,
                    *mint_pubkey,
                    entry,
                    config.ata_create_mode,
                    &mut plan,
                );
            }
            (false, MintIntent::WithBalance { .. }) => {
                return Err(TokenError::WithBalanceNotSupported(*mint_pubkey));
            }
            (true, MintIntent::RequireTokenBalance { .. }) => {
                return Err(TokenError::RequireBalanceForSolNotSupported(*mint_pubkey));
            }
            (false, MintIntent::RequireTokenBalance { amount }) => {
                if entry.ata_account.is_none() {
                    return Err(TokenError::InsufficientBalance {
                        mint: *mint_pubkey,
                        required: *amount,
                        actual: 0,
                    });
                }

                let actual = read_token_balance(entry.ata_address, &entry.ata_account)?;
                if actual < *amount {
                    return Err(TokenError::InsufficientBalance {
                        mint: *mint_pubkey,
                        required: *amount,
                        actual,
                    });
                }
                plan.token_account_addresses
                    .insert(*mint_pubkey, entry.ata_address);
            }
        }
    }

    Ok(plan)
}

fn ensure_ata_exists(
    owner: Pubkey,
    mint_pubkey: Pubkey,
    entry: &MintAndAta,
    ata_create_mode: AtaCreateMode,
    plan: &mut TokenAccountPlan,
) {
    if entry.ata_account.is_none() {
        plan.create_instructions.push(build_create_ata_instruction(
            ata_create_mode,
            &owner,
            &owner,
            &mint_pubkey,
            &entry.mint_account.owner,
        ));
    }
    plan.token_account_addresses
        .insert(mint_pubkey, entry.ata_address);
}

fn handle_wrap_sol(
    owner: Pubkey,
    entry: &MintAndAta,
    required_lamports: u64,
    strategy: WrapSolStrategy,
    rent_exempt_lamports: u64,
    ata_create_mode: AtaCreateMode,
    plan: &mut TokenAccountPlan,
) -> Result<(), TokenError> {
    use spl_token::ID as TOKEN_PROGRAM_ID;

    match strategy {
        WrapSolStrategy::Ata => {
            let existing_balance = read_token_balance(entry.ata_address, &entry.ata_account)?;
            let ata_did_not_exist = entry.ata_account.is_none();

            if ata_did_not_exist {
                plan.create_instructions.push(build_create_ata_instruction(
                    ata_create_mode,
                    &owner,
                    &owner,
                    &native_mint::ID,
                    &TOKEN_PROGRAM_ID,
                ));
            }

            if existing_balance < required_lamports {
                let delta = required_lamports - existing_balance;
                plan.create_instructions.push(system_instruction::transfer(
                    &owner,
                    &entry.ata_address,
                    delta,
                ));
                plan.create_instructions.push(
                    sync_native(&TOKEN_PROGRAM_ID, &entry.ata_address)
                        .map_err(|e| TokenError::InstructionBuild(format!("sync_native: {e}")))?,
                );
            }

            if ata_did_not_exist {
                plan.cleanup_instructions.push(
                    close_account(&TOKEN_PROGRAM_ID, &entry.ata_address, &owner, &owner, &[])
                        .map_err(|e| TokenError::InstructionBuild(format!("close_account: {e}")))?,
                );
            }

            plan.token_account_addresses
                .insert(native_mint::ID, entry.ata_address);
        }
        WrapSolStrategy::Keypair => {
            let kp = Keypair::new();
            let token_account_pubkey = kp.pubkey();
            let lamports = required_lamports + rent_exempt_lamports;

            plan.create_instructions
                .push(system_instruction::create_account(
                    &owner,
                    &token_account_pubkey,
                    lamports,
                    {
                        use solana_program_pack::Pack;
                        spl_token::state::Account::LEN as u64
                    },
                    &TOKEN_PROGRAM_ID,
                ));
            plan.create_instructions.push(
                initialize_account3(
                    &TOKEN_PROGRAM_ID,
                    &token_account_pubkey,
                    &native_mint::ID,
                    &owner,
                )
                .map_err(|e| TokenError::InstructionBuild(format!("initialize_account3: {e}")))?,
            );
            plan.cleanup_instructions.push(
                close_account(
                    &TOKEN_PROGRAM_ID,
                    &token_account_pubkey,
                    &owner,
                    &owner,
                    &[],
                )
                .map_err(|e| TokenError::InstructionBuild(format!("close_account: {e}")))?,
            );
            plan.token_account_addresses
                .insert(native_mint::ID, token_account_pubkey);
            plan.additional_signers.push(kp);
        }
        WrapSolStrategy::None => return Err(TokenError::IncoherentWrapStrategy),
    }
    Ok(())
}

fn read_token_balance(
    token_account_pubkey: Pubkey,
    account: &Option<Account>,
) -> Result<u64, TokenError> {
    match account {
        None => Ok(0),
        Some(acc) => {
            use spl_token_2022_interface::{
                extension::StateWithExtensions, state::Account as TokenAccount,
            };

            StateWithExtensions::<TokenAccount>::unpack(&acc.data)
                .map(|a| a.base.amount)
                .map_err(|e| TokenError::TokenAccountDecodeFailed {
                    token_account: token_account_pubkey,
                    reason: format!("token account unpack: {e}"),
                })
        }
    }
}

#[cfg(test)]
mod tests {
    use solana_program_pack::Pack;
    use spl_token::state::Account as SplTokenAccount;

    use super::*;

    fn empty_plan() -> TokenAccountPlan {
        TokenAccountPlan {
            create_instructions: vec![],
            cleanup_instructions: vec![],
            additional_signers: vec![],
            token_account_addresses: HashMap::new(),
        }
    }

    fn mint_account(token_program: Pubkey) -> Account {
        Account {
            lamports: 1_000_000,
            data: vec![0u8; spl_token::state::Mint::LEN],
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
            state: spl_token::state::AccountState::Initialized,
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

    fn wsol_entry(ata_account: Option<Account>) -> MintAndAta {
        MintAndAta {
            mint_account: Account {
                lamports: 1,
                data: vec![],
                owner: spl_token::ID,
                executable: false,
                rent_epoch: 0,
            },
            ata_address: Pubkey::new_unique(),
            ata_account,
        }
    }

    #[test]
    fn ensure_ata_exists_pushes_instruction_when_missing() {
        let owner = Pubkey::new_unique();
        let mint_pubkey = Pubkey::new_unique();
        let entry = MintAndAta {
            mint_account: mint_account(spl_token::ID),
            ata_address: Pubkey::new_unique(),
            ata_account: None,
        };
        let mut plan = empty_plan();
        ensure_ata_exists(
            owner,
            mint_pubkey,
            &entry,
            AtaCreateMode::Idempotent,
            &mut plan,
        );
        assert_eq!(plan.create_instructions.len(), 1);
        assert_eq!(
            plan.token_account_addresses[&mint_pubkey],
            entry.ata_address
        );
    }

    #[test]
    fn read_token_balance_returns_amount_for_valid_account() {
        let pk = Pubkey::new_unique();
        assert_eq!(
            read_token_balance(pk, &Some(token_account_with_amount(1_500_000))).unwrap(),
            1_500_000
        );
    }

    #[test]
    fn ata_missing_creates_transfers_syncs_and_closes_on_cleanup() {
        let owner = Pubkey::new_unique();
        let entry = wsol_entry(None);
        let mut plan = empty_plan();
        handle_wrap_sol(
            owner,
            &entry,
            1_500_000_000,
            WrapSolStrategy::Ata,
            2_039_280,
            AtaCreateMode::Idempotent,
            &mut plan,
        )
        .unwrap();
        assert_eq!(plan.create_instructions.len(), 3);
        assert_eq!(plan.cleanup_instructions.len(), 1);
    }

    #[test]
    fn keypair_creates_ephemeral_account() {
        let owner = Pubkey::new_unique();
        let entry = wsol_entry(None);
        let mut plan = empty_plan();
        handle_wrap_sol(
            owner,
            &entry,
            1_500_000_000,
            WrapSolStrategy::Keypair,
            2_039_280,
            AtaCreateMode::Idempotent,
            &mut plan,
        )
        .unwrap();
        assert_eq!(plan.create_instructions.len(), 2);
        assert_eq!(plan.cleanup_instructions.len(), 1);
        assert_eq!(plan.additional_signers.len(), 1);
        assert_eq!(
            plan.token_account_addresses[&native_mint::ID],
            plan.additional_signers[0].pubkey()
        );
    }

    #[test]
    fn none_strategy_returns_incoherent_error() {
        let owner = Pubkey::new_unique();
        let entry = wsol_entry(None);
        let mut plan = empty_plan();
        let err = handle_wrap_sol(
            owner,
            &entry,
            1,
            WrapSolStrategy::None,
            0,
            AtaCreateMode::Idempotent,
            &mut plan,
        )
        .unwrap_err();
        assert!(matches!(err, TokenError::IncoherentWrapStrategy));
    }

    #[test]
    fn non_sol_with_balance_returns_with_balance_not_supported() {
        let owner = Pubkey::new_unique();
        let mint = Pubkey::new_unique();
        let mut mints = HashMap::new();
        mints.insert(
            mint,
            MintAndAta {
                mint_account: mint_account(spl_token::ID),
                ata_address: Pubkey::new_unique(),
                ata_account: None,
            },
        );
        let state = TokenAccountState { owner, mints };
        let intent = TokenAccountIntent {
            mints: HashMap::from([(mint, MintIntent::WithBalance { lamports: 100 })]),
        };
        let err = prepare_token_accounts(&state, &intent, TokenAccountPlanConfig::with_rent(0))
            .unwrap_err();
        assert!(matches!(err, TokenError::WithBalanceNotSupported(m) if m == mint));
    }

    #[test]
    fn mixed_mints_emit_instructions_in_pubkey_sort_order() {
        let owner = Pubkey::new_unique();
        let mint_low = Pubkey::new_from_array([0x11; 32]);
        let mint_high = Pubkey::new_from_array([0xEE; 32]);
        let entry_low = MintAndAta {
            mint_account: mint_account(spl_token::ID),
            ata_address: Pubkey::new_from_array([0x22; 32]),
            ata_account: None,
        };
        let entry_high = MintAndAta {
            mint_account: mint_account(spl_token::ID),
            ata_address: Pubkey::new_from_array([0xDD; 32]),
            ata_account: None,
        };
        let mut state_mints = HashMap::new();
        state_mints.insert(mint_high, entry_high);
        state_mints.insert(mint_low, entry_low);
        let state = TokenAccountState {
            owner,
            mints: state_mints,
        };
        let intent = TokenAccountIntent {
            mints: HashMap::from([
                (mint_high, MintIntent::EnsureAtaExists),
                (mint_low, MintIntent::EnsureAtaExists),
            ]),
        };

        let mut last_serialized: Option<Vec<Vec<u8>>> = None;
        for _ in 0..5 {
            let plan =
                prepare_token_accounts(&state, &intent, TokenAccountPlanConfig::with_rent(0))
                    .unwrap();
            assert_eq!(plan.create_instructions.len(), 2);
            let serialized: Vec<Vec<u8>> = plan
                .create_instructions
                .iter()
                .map(|ix| ix.data.clone())
                .collect();
            if let Some(prev) = last_serialized.as_ref() {
                assert_eq!(prev, &serialized, "non-deterministic across runs");
            }
            last_serialized = Some(serialized);
        }
    }
}
