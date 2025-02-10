use crate::state::server::{SqLiteBlockStore, SqLiteTransactionPool};
use crate::{
    consensus::logic::{current_round, evaluate_commitment, get_committing_validator},
    crypto::ecdsa::deserialize_vk,
    handlers::handle_block_proposal,
    state::server::{BlockStore, InMemoryConsensus, TransactionPool},
    types::{Block, ConsensusCommitment, Transaction},
    ServerState,
};
use axum::{extract::Path, Extension, Json};
use colored::Colorize;
use k256::ecdsa::signature::Verifier;
use k256::ecdsa::Signature;
use l2_sequencer::config::consensus::ROUND_DURATION;
use patricia_trie::store::types::Node;
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};
pub async fn schedule(
    Extension(_): Extension<Arc<RwLock<ServerState>>>,
    Extension(_): Extension<Arc<RwLock<BlockStore>>>,
    Extension(shared_pool_state): Extension<Arc<Mutex<TransactionPool>>>,
    Extension(_): Extension<Arc<RwLock<InMemoryConsensus>>>,
    Json(transaction): Json<Transaction>,
) -> String {
    let mut shared_pool_lock = shared_pool_state.lock().await;
    let success_response =
        format!("[Ok] Transaction is being sequenced: {:?}", &transaction).to_string();
    shared_pool_lock.insert_transaction(transaction);
    success_response
}
pub async fn commit(
    Extension(_): Extension<Arc<RwLock<ServerState>>>,
    Extension(shared_block_state): Extension<Arc<RwLock<BlockStore>>>,
    Extension(_): Extension<Arc<Mutex<TransactionPool>>>,
    Extension(shared_consensus_state): Extension<Arc<RwLock<InMemoryConsensus>>>,
    Json(commitment): Json<ConsensusCommitment>,
) -> String {
    let block_state_lock = shared_block_state.read().await;
    let mut consensus_state_lock = shared_consensus_state.write().await;
    let success_response = format!("[Ok] Commitment was accepted: {:?}", &commitment).to_string();
    let last_block_unix_timestamp = block_state_lock
        .get_block_by_height(block_state_lock.current_block_height() - 1)
        .timestamp;
    if !consensus_state_lock.round_winner.is_some() {
        // no round winner found, commitment might be valid
        let validator = get_committing_validator(
            last_block_unix_timestamp,
            consensus_state_lock.validators.clone(),
        );
        // todo: check if commitment signature is valid for validator
        if deserialize_vk(&commitment.validator) == validator {
            let winner = evaluate_commitment(commitment, consensus_state_lock.validators.clone());
            consensus_state_lock.round_winner = Some(winner);
        }
    } else {
        println!(
            "[Info] Round Winner: {:?}",
            &consensus_state_lock.round_winner
        );
    }
    success_response
}
pub async fn propose(
    Extension(shared_state): Extension<Arc<RwLock<ServerState>>>,
    Extension(shared_block_state): Extension<Arc<RwLock<BlockStore>>>,
    Extension(_): Extension<Arc<Mutex<TransactionPool>>>,
    Extension(shared_consensus_state): Extension<Arc<RwLock<InMemoryConsensus>>>,
    Json(mut proposal): Json<Block>,
) -> String {
    let block_state_lock = shared_block_state.read().await;
    let mut consensus_state_lock = shared_consensus_state.write().await;
    let last_block_unix_timestamp = block_state_lock
        .get_block_by_height(block_state_lock.current_block_height() - 1)
        .timestamp;
    let error_response = format!("Block was rejected: {:?}", &proposal).to_string();
    let round = current_round(last_block_unix_timestamp);
    if proposal.timestamp < last_block_unix_timestamp + ((round - 1) * (ROUND_DURATION)) {
        println!(
            "[Warning] Invalid Proposal Timestamp: {}",
            proposal.timestamp
        );
        return error_response;
    };
    let block_signature = proposal
        .signature
        .clone()
        .expect("Block has not been signed!");
    if let Some(round_winner) = consensus_state_lock.round_winner {
        if !block_state_lock.block_exists(proposal.height) {
            let signature_deserialized = Signature::from_slice(&block_signature).unwrap();
            match round_winner.verify(&proposal.to_bytes(), &signature_deserialized) {
                Ok(_) => {
                    let res = handle_block_proposal(
                        &mut shared_state.write().await,
                        &mut shared_block_state.write().await,
                        &mut consensus_state_lock,
                        &mut proposal,
                        error_response,
                    )
                    .await;
                    match res {
                        Some(e) => return e,
                        None => {}
                    }
                }
                Err(_) => {
                    println!(
                        "{}",
                        format_args!(
                            "{} Invalid Signature for Round Winner, Proposal rejected",
                            "[Warning]".yellow(),
                        )
                    );
                    return error_response;
                }
            }
            "[Ok] Block was processed".to_string()
        } else {
            "[Ok] Block was processed".to_string()
        }
    } else {
        "[Warning] Awaiting consensus evaluation".to_string()
    }
}
pub async fn merkle_proof(
    Extension(shared_state): Extension<Arc<RwLock<ServerState>>>,
    Extension(_): Extension<Arc<RwLock<BlockStore>>>,
    Extension(_): Extension<Arc<Mutex<TransactionPool>>>,
    Extension(_): Extension<Arc<RwLock<InMemoryConsensus>>>,
    Json(key): Json<Vec<u8>>,
) -> String {
    let mut state_lock = shared_state.write().await;
    let trie_root = state_lock.merkle_trie_root.clone();
    // todo: make merkle proof fn accept an immutable trie state instance
    let merkle_proof = patricia_trie::merkle::merkle_proof(
        &mut state_lock.merkle_trie_state,
        key,
        Node::Root(trie_root),
    )
    .expect("Failed to get merkle proof!");
    serde_json::to_string(&merkle_proof).unwrap()
}
pub async fn get_pool(
    Extension(_): Extension<Arc<RwLock<ServerState>>>,
    Extension(_): Extension<Arc<RwLock<BlockStore>>>,
    Extension(pool_state): Extension<Arc<Mutex<TransactionPool>>>,
    Extension(_): Extension<Arc<RwLock<InMemoryConsensus>>>,
) -> String {
    let pool_state_lock = pool_state.lock().await;
    {
        format!("{:?}", pool_state_lock.get_all_transactions())
    }
}
pub async fn get_commitments(
    Extension(_): Extension<Arc<RwLock<ServerState>>>,
    Extension(_): Extension<Arc<RwLock<BlockStore>>>,
    Extension(_): Extension<Arc<Mutex<TransactionPool>>>,
    Extension(consensus_state): Extension<Arc<RwLock<InMemoryConsensus>>>,
) -> String {
    let consensus_state_lock = consensus_state.read().await;
    format!("{:?}", consensus_state_lock.commitments)
}
pub async fn get_block(
    Extension(_): Extension<Arc<RwLock<ServerState>>>,
    Extension(shared_block_state): Extension<Arc<RwLock<BlockStore>>>,
    Extension(_): Extension<Arc<Mutex<TransactionPool>>>,
    Extension(_): Extension<Arc<RwLock<InMemoryConsensus>>>,
    Path(height): Path<u32>,
) -> String {
    let block_state_lock = shared_block_state.read().await;
    println!(
        "{}",
        format_args!("{} Peer Requested Block #{}", "[Info]".green(), height)
    );
    let previous_block_height = block_state_lock.current_block_height();
    if previous_block_height < height + 1 {
        "[Warning] Requested Block that does not exist".to_string()
    } else {
        match serde_json::to_string(&block_state_lock.get_block_by_height(height)) {
            Ok(block_json) => block_json,
            Err(e) => e.to_string(),
        }
    }
}
pub async fn get_state_root_hash(
    Extension(shared_state): Extension<Arc<RwLock<ServerState>>>,
    Extension(_): Extension<Arc<RwLock<BlockStore>>>,
    Extension(_): Extension<Arc<Mutex<TransactionPool>>>,
    Extension(_): Extension<Arc<RwLock<InMemoryConsensus>>>,
) -> String {
    let shared_state_lock = shared_state.read().await;
    match serde_json::to_string(&shared_state_lock.merkle_trie_root) {
        Ok(trie_root_json) => trie_root_json,
        Err(e) => e.to_string(),
    }
}
pub async fn get_height(
    Extension(_): Extension<Arc<RwLock<ServerState>>>,
    Extension(shared_block_state): Extension<Arc<RwLock<BlockStore>>>,
    Extension(_): Extension<Arc<Mutex<TransactionPool>>>,
    Extension(_): Extension<Arc<RwLock<InMemoryConsensus>>>,
) -> String {
    let block_state_lock = shared_block_state.read().await;
    let previous_block_height = block_state_lock.current_block_height();
    serde_json::to_string(&previous_block_height).unwrap()
}
