#![allow(unused)]
use naom::constants::MAX_BLOCK_SIZE;
use naom::primitives::asset::Asset;
use naom::primitives::block::Block;
use naom::primitives::block::BlockHeader;
use naom::primitives::transaction::{Transaction, TxIn, TxOut};
use sha3::Digest;

use bincode::{deserialize, serialize};
use bytes::Bytes;
use serde::{Deserialize, Serialize};
use sha3::Sha3_256;
use sodiumoxide::crypto::sign::ed25519::PublicKey;
use std::convert::TryInto;

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

/// The hash information of the current block to mine
#[derive(Default, Clone, Debug, Serialize, Deserialize)]
pub struct HashBlock {
    pub unicorn: String,
    pub merkle_hash: String,
    pub nonce: Vec<u8>,
    pub b_num: u64,
}

impl HashBlock {
    /// Creates a new HashBlock to send for mining
    ///
    /// ### Arguments
    ///
    /// * `unicorn`     - The Unicorn value to send
    /// * `merkle_hash` - The merkle root hash of the transactions in this block
    pub fn new_for_mining(unicorn: String, merkle_hash: String, b_num: u64) -> HashBlock {
        HashBlock {
            unicorn,
            merkle_hash,
            b_num,
            nonce: Vec::new(),
        }
    }

    /// Checks whether a HashBlock's nonce has been set
    pub fn has_nonce(&self) -> bool {
        self.nonce.is_empty()
    }
}
