use crate::configurations::DbMode;
use crate::constants::{
    ADDRESS_KEY, DB_PATH_LIVE, DB_PATH_TEST, FUND_KEY, NETWORK_VERSION, WALLET_PATH,
};
use crate::db_utils::SimpleDb;
use crate::user::ReturnPayment;
use bincode::{deserialize, serialize};
use naom::primitives::asset::TokenAmount;
use naom::primitives::transaction::{OutPoint, Transaction, TxConstructor, TxIn};
use naom::primitives::transaction_utils::construct_payment_tx_ins;
use serde::{Deserialize, Serialize};
use sha3::{Digest, Sha3_256};
use sodiumoxide::crypto::sign;
use sodiumoxide::crypto::sign::{PublicKey, SecretKey};
use std::collections::BTreeMap;
use std::io::Error;
use std::sync::{Arc, Mutex};
use tokio::task;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PaymentAddress {
    pub address: String,
    pub net: u8,
}

/// Data structure for wallet storage
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransactionStore {
    pub address: String,
    pub net: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AddressStore {
    pub public_key: PublicKey,
    pub secret_key: SecretKey,
}

/// A reference to fund stores, where `transactions` contains the hash
/// of the transaction and a `u64` of its holding amount
#[derive(Default, Debug, Clone, Serialize, Deserialize)]
pub struct FundStore {
    pub running_total: TokenAmount,
    pub transactions: BTreeMap<OutPoint, TokenAmount>,
}

#[derive(Debug, Clone)]
pub struct WalletDb {
    pub db: Arc<Mutex<SimpleDb>>,
}

impl WalletDb {
    pub fn new(db_mode: DbMode) -> Self {
        Self {
            db: Arc::new(Mutex::new(Self::new_db(db_mode))),
        }
    }

    /// Creates a new DB instance for a given environment, including construction and
    /// teardown
    ///
    /// ### Arguments
    ///
    /// * `db_mode` - The environment to set the DB up in
    fn new_db(db_mode: DbMode) -> SimpleDb {
        let save_path = match db_mode {
            DbMode::Live => format!("{}/{}", WALLET_PATH, DB_PATH_LIVE),
            DbMode::Test(idx) => format!("{}/{}.{}", WALLET_PATH, DB_PATH_TEST, idx),
            DbMode::InMemory => {
                return SimpleDb::new_in_memory();
            }
        };

        SimpleDb::new_file(save_path).unwrap()
    }

    pub async fn with_seed(self, index: usize, seeds: &[String]) -> Self {
        for seed in seeds {
            let mut it = seed.split('-');
            let seed_idx: usize = it.next().unwrap().parse().unwrap();

            if index == seed_idx {
                let tx_hash: String = it.next().unwrap().parse().unwrap();
                let tx_out_p = OutPoint::new(tx_hash, 0);
                let amount = TokenAmount(it.next().unwrap().parse().unwrap());

                let (address, _) = self.generate_payment_address().await;
                self.save_transaction_to_wallet(tx_out_p.clone(), address)
                    .await
                    .unwrap();
                self.save_payment_to_wallet(tx_out_p, amount).await.unwrap();
            }
        }
        self
    }

    /// Generates a new payment address, saving the related keys to the wallet
    /// TODO: Add static address capability for frequent payments
    ///
    /// ### Arguments
    ///
    /// * `net`     - Network version
    pub async fn generate_payment_address(&self) -> (PaymentAddress, AddressStore) {
        let (public_key, secret_key) = sign::gen_keypair();
        let final_address = construct_address(public_key, NETWORK_VERSION);
        let address_keys = AddressStore {
            public_key,
            secret_key,
        };

        let save_result = self
            .save_address_to_wallet(final_address.address.clone(), address_keys.clone())
            .await;
        if save_result.is_err() {
            panic!("Error writing address to wallet");
        }

        (final_address, address_keys)
    }

    /// Saves an address and its ancestor keys to the wallet
    ///
    /// ### Arguments
    ///
    /// * `address` - Address to save to wallet
    /// * `keys`    - Address-related keys to save
    async fn save_address_to_wallet(
        &self,
        address: String,
        keys: AddressStore,
    ) -> Result<(), Error> {
        let db = self.db.clone();
        Ok(task::spawn_blocking(move || {
            // Wallet DB handling
            let mut db = db.lock().unwrap();
            let mut address_list = get_address_stores(&db);

            // Assign the new address to the store
            address_list.insert(address.clone(), keys);

            // Save to disk
            set_address_stores(&mut db, address_list);
        })
        .await?)
    }

    /// Saves an address and the associated transaction with it to the wallet
    ///
    /// ### Arguments
    ///
    /// * `tx_hash`  - Transaction hash
    /// * `address`  - Transaction Address
    pub async fn save_transaction_to_wallet(
        &self,
        tx_hash: OutPoint,
        address: PaymentAddress,
    ) -> Result<(), Error> {
        let PaymentAddress { address, net } = address;
        let tx_store = TransactionStore { address, net };
        let tx_to_save = Some((tx_hash, tx_store)).into_iter().collect();

        self.save_transactions_to_wallet(tx_to_save).await
    }

    /// Saves an address and the associated transactions with it to the wallet
    ///
    /// ### Arguments
    ///
    /// * `tx_to_save`  - All transactions that are required to be saved to wallet
    pub async fn save_transactions_to_wallet(
        &self,
        tx_to_save: BTreeMap<OutPoint, TransactionStore>,
    ) -> Result<(), Error> {
        let db = self.db.clone();
        Ok(task::spawn_blocking(move || {
            let mut db = db.lock().unwrap();

            for (key, value) in tx_to_save {
                let key = serialize(&key).unwrap();
                let input = serialize(&value).unwrap();
                db.put(&key, &input).unwrap();
            }
        })
        .await?)
    }

    /// Saves a received payment to the local wallet
    ///
    /// ### Arguments
    ///
    /// * `hash`    - Hash of the transaction
    /// * `amount`  - Amount of tokens in the payment
    pub async fn save_payment_to_wallet(
        &self,
        hash: OutPoint,
        amount: TokenAmount,
    ) -> Result<(), Error> {
        let db = self.db.clone();
        Ok(task::spawn_blocking(move || {
            // Wallet DB handling
            let mut db = db.lock().unwrap();
            let mut fund_store = get_fund_store(&db);

            // Update the running total and add the transaction to the tab list
            fund_store.running_total += amount;
            fund_store.transactions.insert(hash, amount);

            println!("Testing payment to wallet");
            set_fund_store(&mut db, fund_store);
        })
        .await?)
    }

    /// Fetches valid TxIns based on the wallet's running total and available unspent
    /// transactions
    ///
    /// TODO: Replace errors here with Error enum types that the Result can return
    /// TODO: Possibly sort addresses found ascending, so that smaller amounts are consumed
    ///
    /// ### Arguments
    ///
    /// * `amount_required` - Amount needed
    pub fn fetch_inputs_for_payment(
        &mut self,
        amount_required: TokenAmount,
    ) -> (Vec<TxIn>, Option<ReturnPayment>) {
        let mut tx_ins = Vec::new();
        let mut return_payment = None;

        // Wallet DB handling
        let mut fund_store = self.get_fund_store();

        // Ensure we have enough funds to proceed with payment
        if fund_store.running_total.0 < amount_required.0 {
            panic!("Not enough funds available for payment!");
        }

        // Start fetching TxIns
        let mut amount_made = TokenAmount(0);
        let tx_hashes: Vec<_> = fund_store.transactions.keys().cloned().collect();

        // Start adding amounts to payment and updating FundStore
        for tx_hash in tx_hashes {
            let current_amount = *fund_store.transactions.get(&tx_hash).unwrap();

            // If we've reached target
            if amount_made == amount_required {
                break;
            }
            // If we've overshot
            else if current_amount + amount_made > amount_required {
                let diff = amount_required - amount_made;

                fund_store.running_total -= current_amount;
                amount_made = amount_required;

                // Add a new return payment transaction
                let return_tx_in = self.construct_tx_in_from_prev_out(tx_hash.clone(), false);
                return_payment = Some(ReturnPayment {
                    tx_in: return_tx_in,
                    amount: current_amount - diff,
                    transaction: Transaction::new(),
                });
            }
            // Else add to used stack
            else {
                amount_made += current_amount;
                fund_store.running_total -= current_amount;
            }

            // Add the new TxIn
            let tx_in = self.construct_tx_in_from_prev_out(tx_hash.clone(), true);
            tx_ins.push(tx_in);

            fund_store.transactions.remove(&tx_hash);
        }

        // Save the updated fund store to disk
        self.set_fund_store(fund_store);

        (tx_ins, return_payment)
    }

    /// Constructs a TxIn from a previous output
    ///
    /// ### Arguments
    ///
    /// * `tx_hash`            - Hash to the output to fetch
    /// * `remove_from_wallet` - Whether to remove txs and address from wallet
    pub fn construct_tx_in_from_prev_out(
        &mut self,
        tx_hash: OutPoint,
        remove_from_wallet: bool,
    ) -> TxIn {
        let mut address_store = self.get_address_stores();
        let tx_store = self.get_transaction_store(&tx_hash);
        
        let needed_store: &AddressStore = address_store.get(&tx_store.address).unwrap();
        let signature = sign::sign_detached(&tx_hash.t_hash.as_bytes(), &needed_store.secret_key);
        
        let tx_const = TxConstructor {
            t_hash: tx_hash.t_hash.clone(),
            prev_n: tx_hash.n,
            signatures: vec![signature],
            pub_keys: vec![needed_store.public_key],
        };
        
        if remove_from_wallet {
            // Update the values in the wallet
            let tx_hash_ser = serialize(&tx_hash).unwrap();
            self.delete_key(&tx_hash_ser);
            
            address_store.remove(&tx_store.address);
            self.set_address_stores(address_store);
        }

        let tx_ins = construct_payment_tx_ins(vec![tx_const]);

        tx_ins[0].clone()
    }

    // Get the wallet fund store
    pub fn get_fund_store(&self) -> FundStore {
        get_fund_store(&self.db.lock().unwrap())
    }

    // Set the wallet fund store
    pub fn set_fund_store(&self, fund_store: FundStore) {
        set_fund_store(&mut self.db.lock().unwrap(), fund_store);
    }

    // Get the wallet address store
    pub fn get_address_stores(&self) -> BTreeMap<String, AddressStore> {
        get_address_stores(&self.db.lock().unwrap())
    }

    // Set the wallet address store
    pub fn set_address_stores(&self, address_store: BTreeMap<String, AddressStore>) {
        set_address_stores(&mut self.db.lock().unwrap(), address_store)
    }

    // Get the wallet transaction store
    pub fn get_transaction_store(&self, tx_hash: &OutPoint) -> TransactionStore {
        get_transaction_store(&self.db.lock().unwrap(), tx_hash)
    }

    pub fn delete_key(&self, key: &[u8]) {
        self.db.lock().unwrap().delete(key).unwrap();
    }

    // Get the wallet addresses
    pub fn get_known_address(&self) -> Vec<String> {
        self.get_address_stores()
            .into_iter()
            .map(|(addr, _)| addr)
            .collect()
    }

    // Get the wallet transaction address
    pub fn get_transaction_address(&self, tx_hash: &OutPoint) -> String {
        self.get_transaction_store(tx_hash).address
    }
}

/// Builds an address from a public key
///
/// ### Arguments
///
/// * `pub_key` - A public key to build an address from
/// * `net`     - Network version
pub fn construct_address(pub_key: PublicKey, net: u8) -> PaymentAddress {
    let first_pubkey_bytes = serialize(&pub_key).unwrap();
    let mut first_hash = Sha3_256::digest(&first_pubkey_bytes).to_vec();

    // TODO: Add RIPEMD

    first_hash.insert(0, net);
    let mut second_hash = Sha3_256::digest(&first_hash).to_vec();
    second_hash.truncate(16);

    PaymentAddress {
        address: hex::encode(second_hash),
        net,
    }
}

// Get the wallet fund store
pub fn get_fund_store(db: &SimpleDb) -> FundStore {
    match db.get(FUND_KEY) {
        Ok(Some(list)) => deserialize(&list).unwrap(),
        Ok(None) => FundStore::default(),
        Err(e) => panic!("Failed to access the wallet database with error: {:?}", e),
    }
}

// Set the wallet fund store
pub fn set_fund_store(db: &mut SimpleDb, fund_store: FundStore) {
    db.put(FUND_KEY, &serialize(&fund_store).unwrap()).unwrap();
}

// Get the wallet address store
pub fn get_address_stores(db: &SimpleDb) -> BTreeMap<String, AddressStore> {
    match db.get(ADDRESS_KEY) {
        Ok(Some(list)) => deserialize(&list).unwrap(),
        Ok(None) => BTreeMap::new(),
        Err(e) => panic!("Error accessing wallet: {:?}", e),
    }
}

// Set the wallet address store
pub fn set_address_stores(db: &mut SimpleDb, address_store: BTreeMap<String, AddressStore>) {
    db.put(ADDRESS_KEY, &serialize(&address_store).unwrap())
        .unwrap();
}

// Get the wallet transaction store
pub fn get_transaction_store(db: &SimpleDb, tx_hash: &OutPoint) -> TransactionStore {
    match db.get(&serialize(&tx_hash).unwrap()) {
        Ok(Some(list)) => deserialize(&list).unwrap(),
        Ok(None) => panic!("Transaction not present in wallet: {:?}", tx_hash),
        Err(e) => panic!("Error accessing wallet: {:?}", e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    /// Creating a valid payment address
    fn should_construct_address_valid() {
        let pk = PublicKey([
            196, 234, 50, 92, 76, 102, 62, 4, 231, 81, 211, 133, 33, 164, 134, 52, 44, 68, 174, 18,
            14, 59, 108, 187, 150, 190, 169, 229, 215, 130, 78, 78,
        ]);
        let addr = construct_address(pk, 0);

        assert_eq!(
            addr,
            PaymentAddress {
                address: "fd86f2230f4fd5bfd9cd882732792279".to_string(),
                net: 0
            }
        );
    }

    #[test]
    /// Creating a payment address of 25 bytes
    fn should_construct_address_valid_length() {
        let pk = PublicKey([
            196, 234, 50, 92, 76, 102, 62, 4, 231, 81, 211, 133, 33, 164, 134, 52, 44, 68, 174, 18,
            14, 59, 108, 187, 150, 190, 169, 229, 215, 130, 78, 78,
        ]);
        let addr = construct_address(pk, 0);

        assert_eq!(addr.address.len(), 32);
    }
}
