use crate::profile::{ProfileContract, SLASH_AMOUNT};
use crate::types::{ContentType, DataKey, Error, GiveawayStatus, HelpRequestStatus};
use soroban_sdk::{contract, contractevent, contractimpl, Address, Env};

/// Number of flags required to automatically suspend content.
pub const FLAG_THRESHOLD: u32 = 10;

#[contract]
pub struct GovernanceContract;

#[contractevent]
pub struct ContentFlagged {
    #[topic]
    content_type: ContentType,
    #[topic]
    target_id: u64,
    user: Address,
    count: u32,
}

#[contractevent]
pub struct ContentAutoSuspended {
    #[topic]
    content_type: ContentType,
    #[topic]
    target_id: u64,
    count: u32,
}

#[contractevent]
pub struct ContentAppealed {
    #[topic]
    target_id: u64,
    user: Address,
}

#[contractimpl]
impl GovernanceContract {
    /// Flag a specific Giveaway or HelpRequest by its type and ID.
    /// Each user may only flag a given content item once.
    pub fn flag_content(
        env: Env,
        user: Address,
        content_type: ContentType,
        target_id: u64,
    ) -> Result<(), Error> {
        // 1. Verify caller signature
        user.require_auth();

        // 2. Prevent duplicate flags from the same user
        let flag_key = DataKey::FlagRecord(content_type, target_id, user.clone());
        if env.storage().persistent().has(&flag_key) {
            return Err(Error::AlreadyFlagged);
        }

        // 3. Record that this user has flagged this ID
        env.storage().persistent().set(&flag_key, &true);

        // 4. Increment the total flag count for this ID
        let count_key = DataKey::FlagCount(content_type, target_id);
        let current: u32 = env.storage().persistent().get(&count_key).unwrap_or(0);
        let new_count = current.checked_add(1).ok_or(Error::ArithmeticOverflow)?;
        env.storage().persistent().set(&count_key, &new_count);

        // 5. Emit "ContentFlagged" event: topics = (name, target_id), data = (user, total_flags)
        ContentFlagged {
            content_type,
            target_id,
            user,
            count: new_count,
        }
        .publish(&env);

        // 5. Circuit breaker: suspend if threshold is reached.
        if new_count >= FLAG_THRESHOLD {
            Self::auto_suspend(&env, content_type, target_id, new_count);
        }

        Ok(())
    }

    /// File an appeal for a suspended content.
    /// Only the creator of the content can file an appeal.
    pub fn file_appeal(env: Env, user: Address, target_id: u64) -> Result<(), Error> {
        user.require_auth();

        let giveaway_key = DataKey::Giveaway(target_id);
        let request_key = DataKey::HelpRequest(target_id);

        // Try Giveaway first
        if let Some(mut giveaway) = env
            .storage()
            .persistent()
            .get::<DataKey, crate::types::Giveaway>(&giveaway_key)
        {
            if giveaway.creator != user {
                return Err(Error::NotCreator);
            }
            if giveaway.status != GiveawayStatus::Suspended {
                return Err(Error::InvalidStatus);
            }
            giveaway.status = GiveawayStatus::UnderAppeal;
            env.storage().persistent().set(&giveaway_key, &giveaway);

            ContentAppealed { target_id, user }.publish(&env);
            return Ok(());
        }

        // Try HelpRequest
        if let Some(mut request) = env
            .storage()
            .persistent()
            .get::<DataKey, crate::types::HelpRequest>(&request_key)
        {
            if request.creator != user {
                return Err(Error::NotCreator);
            }
            if request.status != HelpRequestStatus::Suspended {
                return Err(Error::InvalidStatus);
            }
            request.status = HelpRequestStatus::UnderAppeal;
            env.storage().persistent().set(&request_key, &request);

            ContentAppealed { target_id, user }.publish(&env);
            return Ok(());
        }

        // If neither exists, could be future content or invalid ID.
        // We'll return GiveawayNotFound as a fallback.
        Err(Error::GiveawayNotFound)
    }

    /// Returns the total number of flags for a specific content item.
    pub fn get_flag_count(env: Env, content_type: ContentType, target_id: u64) -> u32 {
        env.storage()
            .persistent()
            .get(&DataKey::FlagCount(content_type, target_id))
            .unwrap_or(0)
    }

    /// Returns whether a user has already flagged a specific content item.
    pub fn has_flagged(env: Env, user: Address, content_type: ContentType, target_id: u64) -> bool {
        env.storage()
            .persistent()
            .has(&DataKey::FlagRecord(content_type, target_id, user))
    }

    // ── internal ──────────────────────────────────────────────────────────────

    /// Try to suspend the content item identified by both type and ID.
    /// Silently skips if the intended item does not exist or is not active.
    /// On successful suspension, slashes the author's reputation by [`SLASH_AMOUNT`].
    fn auto_suspend(env: &Env, content_type: ContentType, target_id: u64, count: u32) {
        let author = match content_type {
            ContentType::Giveaway => {
                let key = DataKey::Giveaway(target_id);
                if let Some(mut giveaway) = env
                    .storage()
                    .persistent()
                    .get::<DataKey, crate::types::Giveaway>(&key)
                {
                    if giveaway.status == GiveawayStatus::Active {
                        let creator = giveaway.creator.clone();
                        giveaway.status = GiveawayStatus::Suspended;
                        env.storage().persistent().set(&key, &giveaway);
                        Some(creator)
                    } else {
                        None
                    }
                } else {
                    None
                }
            }
            ContentType::HelpRequest => {
                let key = DataKey::HelpRequest(target_id);
                if let Some(mut request) = env
                    .storage()
                    .persistent()
                    .get::<DataKey, crate::types::HelpRequest>(&key)
                {
                    if request.status == HelpRequestStatus::Open {
                        let creator = request.creator.clone();
                        request.status = HelpRequestStatus::Suspended;
                        env.storage().persistent().set(&key, &request);
                        Some(creator)
                    } else {
                        None
                    }
                } else {
                    None
                }
            }
        };

        if let Some(creator) = author {
            ProfileContract::slash_reputation(env, creator, SLASH_AMOUNT);
            ContentAutoSuspended {
                content_type,
                target_id,
                count,
            }
            .publish(env);
        }
    }
}
