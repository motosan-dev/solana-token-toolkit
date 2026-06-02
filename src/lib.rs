//! solana-token-toolkit
//!
//! Token account workflow primitives for Solana applications.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod error;
pub mod mint;
pub mod plan;
pub mod state;

pub use error::TokenError;
pub use mint::{
    detect_transfer_hooks, get_token_mint_metadata, reject_transfer_hook_mints, TokenMintMetadata,
    TransferFee, TransferHookInfo,
};
pub use plan::{
    prepare_token_accounts, AtaCreateMode, MintIntent, TokenAccountIntent, TokenAccountPlan,
    TokenAccountPlanConfig, WrapSolStrategy,
};
#[cfg(feature = "rpc")]
pub use state::{assemble_token_account_state, fetch_token_account_state};
pub use state::{MintAndAta, TokenAccountState};
