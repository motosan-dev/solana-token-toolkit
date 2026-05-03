# Changelog

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
