use crate::{require_admin, DataKey, TreasuryContract};
use multisig::{meets_full_quorum, record_approval, require_authorized_signer, TreasuryError};
use soroban_sdk::{contractimpl, token, Address, Env, Symbol, Vec};

/// Default withdrawal window duration (24h), used when no window has been configured yet.
pub(crate) const DEFAULT_WINDOW_DURATION: u64 = 24 * 60 * 60;

#[contractimpl]
impl TreasuryContract {
    /// Sets the per-window withdrawal limit (admin-only). A limit of `0` disables limit
    /// enforcement. Emits `withdrawal_limit_updated` with the old and new values so indexers
    /// can track security-relevant changes to withdrawal policy.
    /// Emits: `withdrawal_limit_updated`.
    pub fn set_withdrawal_limit(env: Env, admin: Address, new_limit: i128) {
        require_admin(&env, &admin);
        let old_limit: i128 = env
            .storage()
            .instance()
            .get(&DataKey::WithdrawalLimit)
            .unwrap_or(0);
        env.storage()
            .instance()
            .set(&DataKey::WithdrawalLimit, &new_limit);
        env.events().publish(
            (Symbol::new(&env, "withdrawal_limit_updated"),),
            (old_limit, new_limit),
        );
    }

    /// Returns the currently configured per-window withdrawal limit, or `0` if unset/disabled.
    pub fn get_withdrawal_limit(env: Env) -> i128 {
        env.storage()
            .instance()
            .get(&DataKey::WithdrawalLimit)
            .unwrap_or(0)
    }

    /// Emergency drain of the treasury's full balance of `token_contract` to `recipient`.
    /// Unlike `withdraw_all`, this does NOT require the contract to be paused, but it DOES
    /// require full quorum: every currently active signer's weight must have approved, not
    /// just the configured threshold. Approvals from signers later removed from the registry
    /// do not count towards quorum.
    /// Panics: `UnauthorizedSigner`.
    /// Errors: `QuorumNotMet`.
    /// Emits: `emergency_withdraw_proposed`; `emergency_withdraw_executed` once full quorum is met.
    pub fn propose_emergency_withdraw(
        env: Env,
        signer: Address,
        token_contract: Address,
        recipient: Address,
    ) -> u64 {
        require_authorized_signer(&env, &signer);
        let count: u64 = env
            .storage()
            .instance()
            .get(&DataKey::EmergencyWithdrawCount)
            .unwrap_or(0);
        let id = count + 1;
        let mut approvals = Vec::new(&env);
        let mut weight = 0u32;
        record_approval(&env, &mut approvals, &mut weight, &signer);
        let proposal = EmergencyWithdrawProposal {
            id,
            token_contract,
            recipient,
            approvals,
            approval_weight: weight,
            executed: false,
        };
        env.storage()
            .persistent()
            .set(&DataKey::EmergencyWithdraw(id), &proposal);
        env.storage()
            .instance()
            .set(&DataKey::EmergencyWithdrawCount, &id);
        env.events().publish(
            (Symbol::new(&env, "emergency_withdraw_proposed"), id),
            proposal.approval_weight,
        );
        id
    }

    /// Approves a pending emergency withdrawal. Executes the drain only once FULL quorum
    /// (sum of weights of all currently active signers) is reached -- threshold alone is not
    /// sufficient.
    /// Panics: `UnauthorizedSigner`, `EmergencyWithdrawNotFound`, `AlreadyExecuted`.
    /// Errors: `QuorumNotMet` (returned, not executed, when quorum isn't yet met).
    /// Emits: `emergency_withdraw_approved`; additionally `emergency_withdraw_executed` on full quorum.
    pub fn approve_emergency_withdraw(
        env: Env,
        signer: Address,
        proposal_id: u64,
    ) -> Result<bool, TreasuryError> {
        require_authorized_signer(&env, &signer);
        let mut proposal: EmergencyWithdrawProposal = env
            .storage()
            .persistent()
            .get(&DataKey::EmergencyWithdraw(proposal_id))
            .unwrap_or_else(|| panic!("EmergencyWithdrawNotFound"));
        if proposal.executed {
            panic!("AlreadyExecuted");
        }
        record_approval(
            &env,
            &mut proposal.approvals,
            &mut proposal.approval_weight,
            &signer,
        );
        let signer_list: Vec<Address> = env
            .storage()
            .instance()
            .get(&DataKey::SignerList)
            .unwrap_or_else(|| Vec::new(&env));
        let executed = meets_full_quorum(proposal.approval_weight, &signer_list, &env);
        if executed {
            proposal.executed = true;
            let treasury = env.current_contract_address();
            let token_client = token::Client::new(&env, &proposal.token_contract);
            let balance = token_client.balance(&treasury);
            if balance > 0 {
                token_client.transfer(&treasury, &proposal.recipient, &balance);
            }
            env.events().publish(
                (
                    Symbol::new(&env, "emergency_withdraw_executed"),
                    proposal_id,
                ),
                proposal.recipient.clone(),
            );
        }
        env.storage()
            .persistent()
            .set(&DataKey::EmergencyWithdraw(proposal_id), &proposal);
        env.events().publish(
            (Symbol::new(&env, "emergency_withdraw_approved"), proposal_id),
            proposal.approval_weight,
        );
        if executed {
            Ok(true)
        } else {
            Err(TreasuryError::QuorumNotMet)
        }
    }

    /// Returns a pending (or executed) emergency withdraw proposal by id.
    pub fn get_emergency_withdraw(env: Env, proposal_id: u64) -> EmergencyWithdrawProposal {
        env.storage()
            .persistent()
            .get(&DataKey::EmergencyWithdraw(proposal_id))
            .unwrap_or_else(|| panic!("EmergencyWithdrawNotFound"))
    }
}

use soroban_sdk::contracttype;

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EmergencyWithdrawProposal {
    pub id: u64,
    pub token_contract: Address,
    pub recipient: Address,
    pub approvals: Vec<Address>,
    pub approval_weight: u32,
    pub executed: bool,
}
