//! App to run a compute node.

use async_std::task;
use clap::{App, Arg};
use naom::primitives::transaction_utils::{
    construct_payment_tx, construct_payment_tx_ins, construct_tx_hash,
};
use naom::primitives::{
    asset::Asset,
    transaction::{Transaction, TxConstructor},
};
use sodiumoxide::crypto::sign;
use std::collections::BTreeMap;
use std::{thread, time};
use system::configurations::ComputeNodeConfig;
use system::{ComputeInterface, ComputeNode, Response};

use config;
use std::collections::HashMap;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();

    let matches = App::new("Zenotta Compute Node")
        .about("Runs a basic compute node.")
        .arg(
            Arg::with_name("config")
                .long("config")
                .short("c")
                .help("Run the compute node using the given config file.")
                .takes_value(true),
        )
        .arg(
            Arg::with_name("index")
                .short("i")
                .long("index")
                .help("Run the specified compute node index from config file")
                .takes_value(true),
        )
        .get_matches();

    let config = {
        let mut settings = config::Config::default();
        let setting_file = matches
            .value_of("config")
            .unwrap_or("src/bin/node_settings.toml");

        settings
            .merge(config::File::with_name(setting_file))
            .unwrap();

        let mut config: ComputeNodeConfig = settings.try_into().unwrap();
        if let Some(index) = matches.value_of("index") {
            config.compute_node_idx = index.parse().unwrap();
        }
        config
    };
    println!("Start node with config {:?}", config);
    let node = ComputeNode::new(config).await?;

    println!("Started node at {}", node.address());

    // REQUEST HANDLING
    tokio::spawn({
        let mut node = node.clone();

        // Kick off with fake transactions
        {
            let (pk, sk) = sign::gen_keypair();
            let t_hash = vec![0, 0, 0];
            let signature = sign::sign_detached(&hex::encode(t_hash.clone()).as_bytes(), &sk);

            let tx_const = TxConstructor {
                t_hash: hex::encode(t_hash),
                prev_n: 0,
                b_hash: hex::encode(vec![0]),
                signatures: vec![signature],
                pub_keys: vec![pk],
            };
            let tx_const_t_hash = tx_const.t_hash.clone();

            let tx_ins = construct_payment_tx_ins(vec![tx_const]);
            let payment_tx = construct_payment_tx(
                tx_ins,
                hex::encode(vec![0, 0, 0]),
                None,
                None,
                Asset::Token(4),
                4,
            );

            println!("");
            println!("Getting hash");
            println!("");

            let t_hash = construct_tx_hash(&payment_tx);

            let mut transactions = BTreeMap::new();
            transactions.insert(t_hash, payment_tx);

            let mut seed_uxto = BTreeMap::new();
            seed_uxto.insert(tx_const_t_hash, Transaction::new());
            node.seed_uxto_set(seed_uxto);

            let resp = node.receive_transactions(transactions);
            println!("initial receive_transactions Response: {:?}", resp);
        }

        let storage_connected = {
            let result = node.connect_to_storage().await;
            println!("Storage connection: {:?}", result);
            result.is_ok()
        };

        async move {
            while let Some(response) = node.handle_next_event().await {
                println!("Response: {:?}", response);

                match response {
                    Ok(Response {
                        success: true,
                        reason: "Partition request received successfully",
                    }) => {
                        let _flood = node.flood_rand_num_to_requesters().await.unwrap();
                    }
                    Ok(Response {
                        success: true,
                        reason: "Partition list is full",
                    }) => {
                        let _list_flood = node.flood_list_to_partition().await.unwrap();
                        node.partition_list = Vec::new();

                        let _block_flood = node.flood_block_to_partition().await.unwrap();
                    }
                    Ok(Response {
                        success: true,
                        reason: "Received PoW successfully",
                    }) => {
                        if storage_connected && node.has_current_block() {
                            println!("Send Block to strage");
                            println!("CURRENT BLOCK: {:?}", node.current_block);
                            let _write_to_store = node.send_block_to_storage().await.unwrap();
                        }
                        let _flood = node.flood_block_found_notification().await.unwrap();
                    }
                    Ok(Response {
                        success: true,
                        reason: "All transactions successfully added to tx pool",
                    }) => {
                        println!("Transactions received and processed successfully");
                        println!("CURRENT BLOCK: {:?}", node.clone().current_block);
                    }
                    Ok(Response {
                        success: true,
                        reason: &_,
                    }) => {
                        println!("UNHANDLED RESPONSE TYPE: {:?}", response.unwrap().reason);
                    }
                    Ok(Response {
                        success: false,
                        reason: &_,
                    }) => {
                        println!("WARNING: UNHANDLED RESPONSE TYPE FAILURE");
                    }
                    Err(error) => {
                        panic!("ERROR HANDLING RESPONSE: {:?}", error);
                    }
                }
            }
        }
    });

    loop {}
}
