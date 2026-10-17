use crate::admin::DEFAULT_WINDOW_DURATION;
use crate::{require_admin, require_not_paused, DataKey, TreasuryContract};
use soroban_sdk::{contractimpl, token, Address, Env, Symbol, Vec};

#[contractimpl]
impl TreasuryContract {
    /// Deposits `amount` tokens from `from` into the treasury via `token_contract`.
    /// Panics: `ContractPaused`, `InvalidAmount`.
    /// Emits: `deposit`.
    pub fn deposit(env: Env, from: Address, token_contract: Address, amount: i128) {
        require_not_paused(&env);
        from.require_auth();
        deposit_one(&env, &from, &token_contract, amount);
    }

    /// Deposits multiple `(token_contract, amount)` pairs from `from` into the treasury.
    /// Panics: `ContractPaused`, `InvalidAmount`.
    /// Emits: `deposit` for each deposited token.
    pub fn batch_deposit(env: Env, from: Address, deposits: Vec<(Address, i128)>) {
        require_not_paused(&env);
        from.require_auth();
        for (token_contract, amount) in deposits.iter() {
            deposit_one(&env, &from, &token_contract, amount);
        }
    }

    /// Withdraws `amount` tokens from the treasury to `to` via `token_contract`.
    /// Panics: `ContractPaused`, `InvalidAmount`, `InsufficientBalance`, `DestinationNotAllowed`.
    /// Emits: `withdraw`.
    pub fn withdraw(env: Env, to: Address, token_contract: Address, amount: i128) {
        require_not_paused(&env);
        to.require_auth();
        if amount <= 0 {
            panic!("InvalidAmount");
        }
        // Check withdrawal destination allowlist: if non-empty, `to` must be present.
        let allowlist: Vec<Address> = env
            .storage()
            .instance()
            .get(&DataKey::WithdrawalAllowlist)
            .unwrap_or_else(|| Vec::new(&env));
        if !allowlist.is_empty() && !allowlist.contains(&to) {
            panic!("DestinationNotAllowed");
        }
        let mut balance: i128 = env
            .storage()
            .persistent()
            .get(&DataKey::Balance(to.clone()))
            .unwrap_or(0);
        if balance < amount {
            panic!("InsufficientBalance");
        }
        balance -= amount;
        env.storage()
            .persistent()
            .set(&DataKey::Balance(to.clone()), &balance);

        let withdrawn_in_window = record_withdrawal_in_window(&env, &to, amount);
        let limit: i128 = env
            .storage()
            .instance()
            .get(&DataKey::WithdrawalLimit)
            .unwrap_or(0);
        if limit > 0 && withdrawn_in_window > limit {
            panic!("WithdrawalLimitExceeded");
        }

        let treasury = env.current_contract_address();
        let token_client = token::Client::new(&env, &token_contract);
        token_client.transfer(&treasury, &to, &amount);
        env.events()
            .publish((Symbol::new(&env, "withdraw"), to), amount);
    }

    /// Returns `(withdrawn, window_start, remaining)` for `address`'s current withdrawal
    /// window. `withdrawn` is the amount withdrawn since `window_start`; if the window has
    /// already rolled over (i.e. more than the window duration has elapsed), this returns
    /// `(0, current_timestamp, limit)` reflecting a fresh window without mutating storage.
    /// `remaining` is `limit - withdrawn`, saturating at 0; if no limit is configured,
    /// `remaining` equals `i128::MAX`.
    /// Read-only, no authentication required.
    pub fn get_withdrawn_in_window(env: Env, address: Address) -> (i128, u64, i128) {
        let limit: i128 = env
            .storage()
            .instance()
            .get(&DataKey::WithdrawalLimit)
            .unwrap_or(0);
        let now = env.ledger().timestamp();
        let duration: u64 = env
            .storage()
            .instance()
            .get(&DataKey::WithdrawalWindowDuration)
            .unwrap_or(DEFAULT_WINDOW_DURATION);
        let window_start: u64 = env
            .storage()
            .persistent()
            .get(&DataKey::WithdrawalWindowStart(address.clone()))
            .unwrap_or(now);
        let rolled_over = now.saturating_sub(window_start) >= duration;
        let (withdrawn, effective_start) = if rolled_over {
            (0i128, now)
        } else {
            let withdrawn: i128 = env
                .storage()
                .persistent()
                .get(&DataKey::WithdrawnInWindow(address))
                .unwrap_or(0);
            (withdrawn, window_start)
        };
        let remaining = if limit > 0 {
            let r = limit - withdrawn;
            if r < 0 {
                0
            } else {
                r
            }
        } else {
            i128::MAX
        };
        (withdrawn, effective_start, remaining)
    }

    /// Returns the recorded deposit balance for `address`, or 0 if never deposited.
    /// Read-only, no authentication required.
    pub fn get_balance(env: Env, address: Address) -> i128 {
        env.storage()
            .persistent()
            .get(&DataKey::Balance(address))
            .unwrap_or(0)
    }

    /// Drains the full token balance of the treasury to `recipient` (admin-only, paused-only emergency drain).
    /// Panics: `Unauthorized`, `NotPaused`.
    /// Emits: `treasury_drained`.
    pub fn withdraw_all(env: Env, admin: Address, token_contract: Address, recipient: Address) {
        require_admin(&env, &admin);
        let paused: bool = env
            .storage()
            .instance()
            .get(&DataKey::Paused)
            .unwrap_or(false);
        if !paused {
            panic!("NotPaused");
        }
        let treasury = env.current_contract_address();
        let token_client = token::Client::new(&env, &token_contract);
        let balance = token_client.balance(&treasury);
        if balance > 0 {
            token_client.transfer(&treasury, &recipient, &balance);
        }
        env.events()
            .publish((Symbol::new(&env, "treasury_drained"),), recipient);
    }
}

/// Rolls the withdrawal window for `address` over if it has expired, then adds `amount` to
/// the running total for the (possibly fresh) window. Returns the updated withdrawn-in-window
/// total.
fn record_withdrawal_in_window(env: &Env, address: &Address, amount: i128) -> i128 {
    let now = env.ledger().timestamp();
    let duration: u64 = env
        .storage()
        .instance()
        .get(&DataKey::WithdrawalWindowDuration)
        .unwrap_or(DEFAULT_WINDOW_DURATION);
    let window_start: u64 = env
        .storage()
        .persistent()
        .get(&DataKey::WithdrawalWindowStart(address.clone()))
        .unwrap_or(now);
    let rolled_over = now.saturating_sub(window_start) >= duration;
    let (mut withdrawn, start) = if rolled_over {
        (0i128, now)
    } else {
        let withdrawn: i128 = env
            .storage()
            .persistent()
            .get(&DataKey::WithdrawnInWindow(address.clone()))
            .unwrap_or(0);
        (withdrawn, window_start)
    };
    withdrawn += amount;
    env.storage()
        .persistent()
        .set(&DataKey::WithdrawalWindowStart(address.clone()), &start);
    env.storage()
        .persistent()
        .set(&DataKey::WithdrawnInWindow(address.clone()), &withdrawn);
    withdrawn
}

fn deposit_one(env: &Env, from: &Address, token_contract: &Address, amount: i128) {
    if amount <= 0 {
        panic!("InvalidAmount");
    }
    let treasury = env.current_contract_address();
    let token_client = token::Client::new(env, token_contract);
    token_client.transfer(from, &treasury, &amount);
    let mut balance: i128 = env
        .storage()
        .persistent()
        .get(&DataKey::Balance(from.clone()))
        .unwrap_or(0);
    balance += amount;
    env.storage()
        .persistent()
        .set(&DataKey::Balance(from.clone()), &balance);
    env.events()
        .publish((Symbol::new(env, "deposit"), from.clone()), amount);
}
