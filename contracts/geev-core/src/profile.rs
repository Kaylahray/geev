use crate::types::{DataKey, Error, ProfileData};
use soroban_sdk::{contract, contractimpl, panic_with_error, Address, Env, String};

/// Reputation deducted from a content author when their item is auto-suspended.
pub const SLASH_AMOUNT: u64 = 5;

/// Reputation points removed per fully elapsed decay period.
pub const DECAY_PER_PERIOD: u64 = 1;

/// Length of one decay period in ledger seconds (30 days).
pub const DECAY_PERIOD_SECONDS: u64 = 30 * 24 * 60 * 60;

#[contract]
pub struct ProfileContract;

#[contractimpl]
impl ProfileContract {
    /// Create or update the caller's on-chain profile.
    ///
    /// * `user`        – the account whose profile is being set; must sign the tx
    /// * `username`    – desired display name (must be unique across all users)
    /// * `avatar_hash` – IPFS CID pointing to the avatar image
    pub fn set_profile(env: Env, user: Address, username: String, avatar_hash: String) {
        user.require_auth();

        let profile_key = DataKey::Profile(user.clone());
        let username_key = DataKey::Username(username.clone());

        // Enforce username uniqueness – reject if another address owns this username
        if let Some(owner) = env
            .storage()
            .persistent()
            .get::<DataKey, Address>(&username_key)
        {
            if owner != user {
                panic_with_error!(&env, Error::UsernameTaken);
            }
        }

        // Free the old username mapping when a user changes their handle
        if let Some(existing) = env
            .storage()
            .persistent()
            .get::<DataKey, ProfileData>(&profile_key)
        {
            if existing.username != username {
                env.storage()
                    .persistent()
                    .remove(&DataKey::Username(existing.username));
            }
        }

        let profile = ProfileData {
            username: username.clone(),
            avatar_hash,
        };

        env.storage().persistent().set(&profile_key, &profile);

        // Reverse mapping: Username → Address
        env.storage().persistent().set(&username_key, &user);
    }

    /// Retrieve profile data for a given wallet address.
    /// Returns `None` if no profile has been registered for that address.
    pub fn get_profile(env: Env, user: Address) -> Option<ProfileData> {
        env.storage()
            .persistent()
            .get::<DataKey, ProfileData>(&DataKey::Profile(user))
    }

    /// Reverse lookup – resolve a username to its owner's address.
    /// Returns `None` if the username is not registered.
    pub fn resolve_username(env: Env, username: String) -> Option<Address> {
        env.storage()
            .persistent()
            .get::<DataKey, Address>(&DataKey::Username(username))
    }

    /// Read the reputation score for a given address (defaults to 0).
    ///
    /// **Decay-on-read:** applies time-based decay before returning, then
    /// persists the decayed score and advances `ReputationUpdatedAt`.
    /// See module-level constants `DECAY_PER_PERIOD` / `DECAY_PERIOD_SECONDS`.
    pub fn get_reputation(env: Env, user: Address) -> u64 {
        Self::apply_decay_and_load(&env, &user)
    }
}

impl ProfileContract {
    /// Apply elapsed decay periods to `user`'s score, persist if changed, return score.
    /// Never goes below zero.
    fn apply_decay_and_load(env: &Env, user: &Address) -> u64 {
        let key = DataKey::Reputation(user.clone());
        let updated_key = DataKey::ReputationUpdatedAt(user.clone());
        let mut score: u64 = env.storage().persistent().get(&key).unwrap_or(0);
        let now = env.ledger().timestamp();
        let last: u64 = env.storage().persistent().get(&updated_key).unwrap_or(now);

        if now > last && DECAY_PERIOD_SECONDS > 0 {
            let periods = (now - last) / DECAY_PERIOD_SECONDS;
            if periods > 0 {
                let decay = periods.saturating_mul(DECAY_PER_PERIOD);
                score = score.saturating_sub(decay);
                env.storage().persistent().set(&key, &score);
                // Advance by whole periods so a partial period is not lost.
                let new_updated = last.saturating_add(periods.saturating_mul(DECAY_PERIOD_SECONDS));
                env.storage().persistent().set(&updated_key, &new_updated);
                return score;
            }
        }

        if !env.storage().persistent().has(&updated_key) {
            env.storage().persistent().set(&updated_key, &now);
        }

        score
    }

    fn write_reputation(env: &Env, user: &Address, score: u64) {
        env.storage()
            .persistent()
            .set(&DataKey::Reputation(user.clone()), &score);
        env.storage().persistent().set(
            &DataKey::ReputationUpdatedAt(user.clone()),
            &env.ledger().timestamp(),
        );
    }

    /// Increment `user`'s reputation by 1.
    /// Private — only callable from within this crate (e.g. `claim_prize`).
    /// Never exposed in the contract ABI.
    pub(crate) fn increment_reputation(env: &Env, user: Address) {
        let score = Self::apply_decay_and_load(env, &user);
        Self::write_reputation(env, &user, score.saturating_add(1));
    }

    /// Reduce `user`'s reputation by `amount`, saturating at zero.
    /// Private — called from governance auto-suspend / appeal restore helpers.
    pub(crate) fn slash_reputation(env: &Env, user: Address, amount: u64) {
        let score = Self::apply_decay_and_load(env, &user);
        Self::write_reputation(env, &user, score.saturating_sub(amount));
    }

    /// Restore reputation previously removed by a slash (saturating add).
    pub(crate) fn restore_reputation(env: &Env, user: Address, amount: u64) {
        let score = Self::apply_decay_and_load(env, &user);
        Self::write_reputation(env, &user, score.saturating_add(amount));
    }
}
