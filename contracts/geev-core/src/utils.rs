use crate::types::{DataKey, Error, Giveaway};
use soroban_sdk::{panic_with_error, symbol_short, Env, Symbol};

const LOCK_KEY: Symbol = symbol_short!("Lock");

/// Maximum allowed fee in basis points (100% = 10_000 bps).
pub const MAX_FEE_BPS: u32 = 10_000;

/// Default protocol fee when no global, token, or giveaway fee is set (1%).
pub const DEFAULT_FEE_BPS: u32 = 100;

pub fn with_reentrancy_guard<F, T>(env: &Env, f: F) -> T
where
    F: FnOnce() -> T,
{
    if env.storage().temporary().has(&LOCK_KEY) {
        panic!("reentrancy detected");
    }
    env.storage().temporary().set(&LOCK_KEY, &true);
    let result = f();
    env.storage().temporary().remove(&LOCK_KEY);
    result
}

/// Reject fees above [`MAX_FEE_BPS`].
pub fn validate_fee(env: &Env, fee_bps: u32) {
    if fee_bps > MAX_FEE_BPS {
        panic_with_error!(env, Error::InvalidFee);
    }
}

/// Resolve the effective fee for a giveaway claim.
///
/// Precedence (highest to lowest):
/// 1. Giveaway `fee_bps` override (if `Some`)
/// 2. Per-token fee (`DataKey::TokenFee`)
/// 3. Global fee (`DataKey::Fee`)
/// 4. Default ([`DEFAULT_FEE_BPS`] = 100)
pub fn resolve_fee_bps(env: &Env, giveaway: &Giveaway) -> u32 {
    if let Some(override_fee) = giveaway.fee_bps {
        return override_fee;
    }

    let token_fee_key = DataKey::TokenFee(giveaway.token.clone());
    if let Some(token_fee) = env.storage().instance().get::<DataKey, u32>(&token_fee_key) {
        return token_fee;
    }

    env.storage()
        .instance()
        .get(&DataKey::Fee)
        .unwrap_or(DEFAULT_FEE_BPS)
}
