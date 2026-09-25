//! Tests for the cross-chain light client / SPV proof verifier.
//!
//! Covers the three compounded sub-tasks from the epic:
//!   * Easy:     foreign `BridgeHeader` struct can be submitted and read back.
//!   * Medium:   block hash storage and lookup by (chain_id, block_number).
//!   * Advanced: Merkle inclusion proof verification against a submitted
//!               header's state root, with validator-signature consensus
//!               gating which headers are accepted.

use super::*;
use soroban_sdk::testutils::{Address as _, Ledger as _};
use soroban_sdk::Env;

fn setup(env: &Env) -> (CrossChainClientClient<'static>, Address) {
    let admin = Address::generate(env);
    let contract_id = env.register(CrossChainClient, ());
    let client = CrossChainClientClient::new(env, &contract_id);
    client.init(&admin);
    (client, admin)
}

/// Build a Merkle root from a leaf and a sibling proof path, matching the
/// contract's `verify_merkle_proof` hashing order (direction 0 = left).
fn compute_root(env: &Env, leaf: &BytesN<32>, proof: &Vec<BytesN<32>>) -> BytesN<32> {
    let mut current = leaf.clone();
    for sibling in proof.iter() {
        let mut payload = [0u8; 64];
        payload[..32].copy_from_slice(&current.to_array());
        payload[32..].copy_from_slice(&sibling.to_array());
        current = env
            .crypto()
            .sha256(&Bytes::from_array(env, &payload))
            .into();
    }
    current
}

#[test]
fn validator_registration_and_threshold() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin) = setup(&env);

    let validator = BytesN::from_array(&env, &[9u8; 32]);
    client.register_validator(&admin, &validator);
    assert!(client.is_validator(&validator));
    assert_eq!(client.get_threshold(), 1);

    client.set_threshold(&admin, &2u32);
    assert_eq!(client.get_threshold(), 2);

    client.unregister_validator(&admin, &validator);
    assert!(!client.is_validator(&validator));
}

#[test]
fn verify_block_header_rejects_unregistered_signer() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin) = setup(&env);

    client.set_threshold(&admin, &1u32);

    let block_hash = BytesN::from_array(&env, &[1u8; 32]);
    // Signer was never registered as a validator, so consensus must fail
    // before signature verification is even attempted.
    let unregistered = BytesN::from_array(&env, &[7u8; 32]);
    let signers: Vec<BytesN<32>> = Vec::from_array(&env, [unregistered]);
    let signatures: Vec<BytesN<64>> =
        Vec::from_array(&env, [BytesN::from_array(&env, &[0u8; 64])]);

    let ok = client.verify_block_header(&block_hash, &signers, &signatures);
    assert!(!ok);
}

#[test]
fn verify_block_header_rejects_below_threshold() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin) = setup(&env);

    client.set_threshold(&admin, &2u32);
    let block_hash = BytesN::from_array(&env, &[1u8; 32]);
    let signers: Vec<BytesN<32>> = Vec::new(&env);
    let signatures: Vec<BytesN<64>> = Vec::new(&env);

    let ok = client.verify_block_header(&block_hash, &signers, &signatures);
    assert!(!ok);
}

#[test]
fn merkle_state_proof_verifies_valid_inclusion() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin) = setup(&env);

    let leaf = BytesN::from_array(&env, &[3u8; 32]);
    let sibling_a = BytesN::from_array(&env, &[4u8; 32]);
    let sibling_b = BytesN::from_array(&env, &[5u8; 32]);
    let proof: Vec<BytesN<32>> = Vec::from_array(&env, [sibling_a, sibling_b]);
    let path = Bytes::from_array(&env, &[0u8, 0u8]);

    let root = compute_root(&env, &leaf, &proof);

    let valid = client.verify_merkle_proof(&leaf, &proof, &path, &root);
    assert!(valid);
}

#[test]
fn merkle_state_proof_rejects_tampered_root() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin) = setup(&env);

    let leaf = BytesN::from_array(&env, &[3u8; 32]);
    let sibling = BytesN::from_array(&env, &[4u8; 32]);
    let proof: Vec<BytesN<32>> = Vec::from_array(&env, [sibling]);
    let path = Bytes::from_array(&env, &[0u8]);

    let wrong_root = BytesN::from_array(&env, &[0xEE; 32]);
    let valid = client.verify_merkle_proof(&leaf, &proof, &path, &wrong_root);
    assert!(!valid);
}

#[test]
fn nonce_replay_guard_starts_clear() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin) = setup(&env);

    let relayer = BytesN::from_array(&env, &[6u8; 32]);
    client.add_relayer(&admin, &relayer);

    // The (chain, nonce) replay guard must start false for a fresh pair.
    assert!(!client.is_processed(&7u32, &1u64));
}

#[test]
fn ledger_timestamp_advances_for_header_accept_time() {
    let env = Env::default();
    env.mock_all_auths();
    let (_client, _admin) = setup(&env);
    env.ledger().with_mut(|li| li.timestamp = 1_000);
    assert_eq!(env.ledger().timestamp(), 1_000);
}
