//! Error types returned by the `solana-token-toolkit` public API.

use solana_pubkey::Pubkey;

/// All errors returned by `solana-token-toolkit` public API.
#[derive(thiserror::Error, Debug)]
#[non_exhaustive]
pub enum TokenError {
    /// An RPC call failed.
    #[error("RPC call failed: {0}")]
    Rpc(#[from] solana_client::client_error::ClientError),

    /// A requested mint account does not exist on chain.
    #[error("mint account not found: {0}")]
    MintNotFound(Pubkey),

    /// Failed to decode a mint account.
    #[error("mint account decode failed: {mint}: {reason}")]
    MintDecodeFailed {
        /// The mint pubkey whose account data could not be decoded.
        mint: Pubkey,
        /// Underlying decode error message.
        reason: String,
    },

    /// Failed to decode a token account (e.g. while reading wSOL balance).
    #[error("token account decode failed: {token_account}: {reason}")]
    TokenAccountDecodeFailed {
        /// The token account pubkey whose data could not be decoded.
        token_account: Pubkey,
        /// Underlying decode error message.
        reason: String,
    },

    /// `assemble_token_account_state` was called with mismatched slice lengths.
    #[error("mints/mint_accounts length mismatch: {mints} vs {mint_accounts}")]
    LengthMismatch {
        /// Length of the `mints` slice.
        mints: usize,
        /// Length of the `mint_accounts` slice.
        mint_accounts: usize,
    },

    /// `reject_transfer_hook_mints` found a hook on a mint.
    #[error("transfer hook detected on mint {mint} (program: {program})")]
    TransferHookDetected {
        /// The mint that has the transfer hook.
        mint: Pubkey,
        /// The hook program ID configured on the mint.
        program: Pubkey,
    },

    /// `MintIntent::WithBalance` was used with a non-SOL mint.
    #[error("WithBalance intent on non-SOL mint {0} is not supported")]
    WithBalanceNotSupported(Pubkey),

    /// `MintIntent::WithBalance` for native SOL was paired with
    /// `WrapSolStrategy::None`.
    #[error(
        "WithBalance intent for native SOL requires WrapSolStrategy::Ata or Keypair, not None"
    )]
    IncoherentWrapStrategy,

    /// An `spl-token-*` instruction builder returned an error.
    #[error("instruction build failed: {0}")]
    InstructionBuild(String),

    /// `MintIntent::RequireTokenBalance` failed because actual balance is
    /// below required. `actual` is `0` when the ATA does not exist on chain.
    #[error("token balance insufficient on mint {mint}: required {required}, actual {actual}")]
    InsufficientBalance {
        /// The mint whose balance check failed.
        mint: Pubkey,
        /// The amount the caller required.
        required: u64,
        /// The actual balance found in `state.mints[mint].ata_account` (or 0 if missing).
        actual: u64,
    },

    /// `MintIntent::RequireTokenBalance` was used with `native_mint::ID`
    /// (SOL/wSOL). For SOL balance preparation, use `MintIntent::WithBalance`
    /// (which transfers SOL into wSOL).
    #[error("RequireTokenBalance intent on SOL/wSOL mint {0} is not supported (use WithBalance)")]
    RequireBalanceForSolNotSupported(Pubkey),
}
