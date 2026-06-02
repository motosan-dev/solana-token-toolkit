# Changelog

## v0.3.0 — 2026-06-02

Additive, backward-compatible release.

### Added

- `rpc` cargo feature (**on by default**) gating the `solana-client` dependency.
  Consumers that only need the pure surface — planning (`prepare_token_accounts`,
  `WrapSolStrategy`, …), mint metadata, and the `TokenAccountState` / `MintAndAta`
  *types* — can now depend with `default-features = false` to drop `solana-client`
  (and the QUIC/tokio stack) entirely for an I/O-free build.

### Notes

- Default builds are unchanged: `fetch_token_account_state`,
  `assemble_token_account_state`, and `TokenError::Rpc` remain available under the
  default `rpc` feature. Only `default-features = false` consumers see them gated out.

## v0.2.0 — 2026-05-03

Breaking release addressing architectural gaps surfaced after v0.1.1.

### Breaking Changes

#### `prepare_token_accounts` signature

Before:

```rust
let plan = prepare_token_accounts(&state, &intent, WrapSolStrategy::Ata, rent)?;
```

After:

```rust
let plan = prepare_token_accounts(
    &state,
    &intent,
    TokenAccountPlanConfig::with_rent(rent),
)?;
```

To override defaults:

```rust
let plan = prepare_token_accounts(&state, &intent, TokenAccountPlanConfig {
    ata_create_mode: AtaCreateMode::Legacy,
    ..TokenAccountPlanConfig::with_rent(rent)
})?;
```

`TokenAccountPlanConfig` deliberately has no `Default` impl because `rent_exempt_lamports` has no safe default.

#### `TokenMintWithFee` → `TokenMintMetadata`

Before:

```rust
let parsed = get_token_mint_and_transfer_fee(pk, &acc, epoch)?;
```

After:

```rust
let parsed = get_token_mint_metadata(pk, &acc, epoch)?;
println!(
    "program: {}, fee: {:?}, hook: {:?}",
    parsed.program_id,
    parsed.transfer_fee,
    parsed.transfer_hook_program_id,
);
```

### New Features

- `MintIntent::RequireTokenBalance { amount }` validates non-SOL token balance at plan time.
- `AtaCreateMode::{Idempotent, Legacy}` controls ATA create instruction style.
- `TokenMintMetadata` includes `program_id` and `transfer_hook_program_id`.

### Error Type Additions

- `TokenError::InsufficientBalance { mint, required, actual }`
- `TokenError::RequireBalanceForSolNotSupported(Pubkey)`

## v0.1.1 — 2026-05-02

Test-infrastructure refactor in dependent migration. No public API changes from v0.1.0.

## v0.1.0 — 2026-05-02

Initial release.
