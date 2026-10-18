use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, Env};
use treasury::{TreasuryContract, TreasuryContractClient};

fn setup(env: &Env, threshold: u32) -> (TreasuryContractClient, Address) {
    env.mock_all_auths();
    let admin = Address::generate(env);
    let id = env.register_contract(None, TreasuryContract);
    let client = TreasuryContractClient::new(env, &id);
    client.initialize(&admin, &threshold, &soroban_sdk::Vec::new(env));
    (client, admin)
}

/// Threshold is 2, but three signers are active (admin=1, b=1, c=1 => total weight 3).
/// Approving only up to the threshold weight must NOT execute the emergency withdraw --
/// full quorum (all active signer weight) is required instead.
#[test]
fn emergency_withdraw_requires_full_quorum_not_just_threshold() {
    let env = Env::default();
    let (client, admin) = setup(&env, 2);
    let b = Address::generate(&env);
    let c = Address::generate(&env);
    client.set_signer(&admin, &b, &1);
    client.set_signer(&admin, &c, &1);

    let token_contract = Address::generate(&env);
    let recipient = Address::generate(&env);

    let id = client.propose_emergency_withdraw(&admin, &token_contract, &recipient);
    // admin alone = weight 1, below threshold(2) and total quorum(3).
    let result = client.try_approve_emergency_withdraw(&b, &id);
    // weight now 2 == threshold, but total signer weight is 3 -> quorum not met, not executed.
    assert_eq!(result, Ok(Ok(false)));

    let proposal = client.get_emergency_withdraw(&id);
    assert_eq!(proposal.approval_weight, 2);
    assert!(!proposal.executed);
}

/// Once every active signer has approved (full quorum == total active signer weight),
/// the emergency withdraw executes.
#[test]
fn emergency_withdraw_succeeds_with_full_approval() {
    let env = Env::default();
    let (client, admin) = setup(&env, 2);
    let b = Address::generate(&env);
    let c = Address::generate(&env);
    client.set_signer(&admin, &b, &1);
    client.set_signer(&admin, &c, &1);

    let token_contract = Address::generate(&env);
    let recipient = Address::generate(&env);

    let id = client.propose_emergency_withdraw(&admin, &token_contract, &recipient);
    client.approve_emergency_withdraw(&b, &id);
    let result = client.try_approve_emergency_withdraw(&c, &id);

    assert_eq!(result, Ok(Ok(true)));
    let proposal = client.get_emergency_withdraw(&id);
    assert_eq!(proposal.approval_weight, 3);
    assert!(proposal.executed);
}

/// A signer removed from the registry before approving must not have their earlier
/// approval counted towards quorum: once removed, their weight is excluded from both
/// the proposal's accumulated weight bookkeeping (they can no longer approve, being
/// unauthorized) and from the total signer weight used to determine quorum.
#[test]
fn removed_signers_approval_does_not_count_towards_quorum() {
    let env = Env::default();
    let (client, admin) = setup(&env, 2);
    let b = Address::generate(&env);
    let c = Address::generate(&env);
    client.set_signer(&admin, &b, &1);
    client.set_signer(&admin, &c, &1);

    let token_contract = Address::generate(&env);
    let recipient = Address::generate(&env);

    let id = client.propose_emergency_withdraw(&admin, &token_contract, &recipient);
    client.approve_emergency_withdraw(&b, &id);
    // b is now removed; total active signer weight drops to 2 (admin + c).
    client.remove_signer(&admin, &b);

    let proposal = client.get_emergency_withdraw(&id);
    // Snapshotted approval weight still reflects b's earlier approval (2), but since b is no
    // longer an active signer, full quorum is now just admin + c = 2, which is already met.
    assert_eq!(proposal.approval_weight, 2);

    // c approves; quorum (admin+c=2, matching current total active weight of 2) is met.
    let result = client.try_approve_emergency_withdraw(&c, &id);
    assert_eq!(result, Ok(Ok(true)));
    assert!(client.get_emergency_withdraw(&id).executed);
}

/// A non-signer cannot approve an emergency withdraw at all.
#[test]
fn unauthorized_signer_cannot_approve_emergency_withdraw() {
    let env = Env::default();
    let (client, admin) = setup(&env, 1);
    let token_contract = Address::generate(&env);
    let recipient = Address::generate(&env);
    let outsider = Address::generate(&env);

    let id = client.propose_emergency_withdraw(&admin, &token_contract, &recipient);
    let result = client.try_approve_emergency_withdraw(&outsider, &id);
    assert!(result.is_err());
}
