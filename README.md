# solana-token-toolkit

Token account workflow primitives for Solana applications.

[![Crates.io](https://img.shields.io/crates/v/solana-token-toolkit.svg)](https://crates.io/crates/solana-token-toolkit)
[![Docs](https://docs.rs/solana-token-toolkit/badge.svg)](https://docs.rs/solana-token-toolkit)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#license)

> **Status:** v0.2. API is stable within minor versions.

## What this is

`solana-token-toolkit` provides three tiers of token-account utilities:

1. **State** (`fetch_token_account_state`, `assemble_token_account_state`) — query the on-chain state of an owner's ATAs across a set of mints
2. **Mint extensions** (`get_token_mint_metadata`, `detect_transfer_hooks`, `reject_transfer_hook_mints`) — parse Token-2022 transfer fees and transfer hooks
3. **ATA workflow** (`prepare_token_accounts`) — produce instructions to prepare token accounts for a transaction, including wSOL wrapping paths and non-SOL balance validation

It is independent of any DEX or wallet abstractions. No global mutable state. No `anyhow` in public API.

## Installation

```toml
[dependencies]
solana-token-toolkit = "0.3"
# solana-client is pulled in by the default `rpc` feature. For a pure, I/O-free
# build (planning + mint metadata + state types only), use:
#   solana-token-toolkit = { version = "0.3", default-features = false }
# and you can then drop the solana-client line below.
solana-client = "3"
solana-keypair = "3"
solana-pubkey = "4"
solana-signer = "3"
spl-token-interface = "2" # for native_mint::ID in examples
```

Compatible with stable Solana SDK 3.x split crates. MSRV: Rust 1.89.

## Usage

End-to-end example: ensure two token accounts exist (one regular SPL token, one wSOL wrapped from native SOL), then build a transaction.

```rust
use std::collections::HashMap;
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_keypair::Keypair;
use solana_pubkey::Pubkey;
use solana_signer::Signer;
use solana_token_toolkit::*;
use spl_token_interface::native_mint;

# async fn example() -> Result<(), TokenError> {
let rpc = RpcClient::new("https://api.devnet.solana.com".to_string());
let user = Keypair::new();
let usdc_mint: Pubkey = "Gh9ZwEmdLJ8DscKNTkTqPbNwLNNBjuSzaG9Vp2KGtKJr".parse().unwrap();

let mints = vec![native_mint::ID, usdc_mint];
let state = fetch_token_account_state(&rpc, user.pubkey(), &mints).await?;
reject_transfer_hook_mints(&state)?;

let rent = rpc.get_minimum_balance_for_rent_exemption(165).await?;
let intent = TokenAccountIntent {
    mints: HashMap::from([
        (native_mint::ID, MintIntent::WithBalance { lamports: 1_500_000_000 }),
        (usdc_mint, MintIntent::EnsureAtaExists),
    ]),
};

let plan = prepare_token_accounts(
    &state,
    &intent,
    TokenAccountPlanConfig::with_rent(rent),
)?;

// Transaction assembly: plan.create_instructions ++ main instructions ++
// plan.cleanup_instructions. Include plan.additional_signers.
# let _ = plan;
# Ok(())
# }
```

For non-SOL token-balance assertions:

```rust
let mut intent = TokenAccountIntent { mints: HashMap::new() };
intent.mints.insert(usdc_mint, MintIntent::RequireTokenBalance { amount: 1_000_000 });
// prepare_token_accounts returns Err(InsufficientBalance { ... }) if balance is too low.
```

For byte-compat with legacy non-idempotent ATA create:

```rust
let plan = prepare_token_accounts(&state, &intent, TokenAccountPlanConfig {
    ata_create_mode: AtaCreateMode::Legacy,
    ..TokenAccountPlanConfig::with_rent(rent)
})?;
```

For full v0.1 → v0.2 migration steps, see [CHANGELOG.md](CHANGELOG.md).

## Key design decisions

- **No global mutable state** — wrapping and ATA create modes are per-call config.
- **Caller-provided rent** — the library stays synchronous in Tier 3.
- **Deterministic instruction order** — `prepare_token_accounts` sorts intents by mint pubkey.
- **Typed errors** — no `anyhow` in public API.
- **No Seed strategy** — protocol-specific wrapping is intentionally out of scope.
- **Temporary wSOL when created by the toolkit** — `MintIntent::WithBalance` + `WrapSolStrategy::Ata` creates a wSOL ATA if missing, uses it for the main transaction, and includes a cleanup close instruction for that newly-created ATA. Existing user wSOL ATAs are never auto-closed.

## License

Dual-licensed under MIT or Apache-2.0.
