//! App to run a storage node.

use clap::{App, Arg};
use naom::primitives::asset::TokenAmount;
use std::net::SocketAddr;
use system::configurations::UserNodeConfig;
use system::{
    loop_wait_connnect_to_peers_async, loops_re_connect_disconnect, shutdown_connections,
    ResponseResult,
};
use system::{routes, UserNode};

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    let matches = clap_app().get_matches();
    let config = configuration(load_settings(&matches));

    println!("Starting node with config: {:?}", config);
    println!();

    // Handle a payment amount
    let amount_to_send = match matches.value_of("amount").map(|a| a.parse::<u64>()) {
        None => None,
        Some(Ok(v)) => Some(TokenAmount(v)),
        Some(Err(e)) => panic!("Unable to pay with amount specified due to error: {:?}", e),
    };

    let peer_user_node = *config.user_nodes.get(config.peer_user_node_idx).unwrap();
    let mut node = UserNode::new(config, Default::default()).await.unwrap();

    println!("Started node at {}", node.address());

    let (node_conn, addrs_to_connect, expected_connected_addrs) = node.connect_info_peers();
    let local_event_tx = node.local_event_tx().clone();
    let api_inputs = node.api_inputs();

    // PERMANENT CONNEXION/DISCONNECTION HANDLING
    let ((conn_loop_handle, stop_re_connect_tx), (disconn_loop_handle, stop_disconnect_tx)) = {
        let (re_connect, disconnect_test) =
            loops_re_connect_disconnect(node_conn.clone(), addrs_to_connect, local_event_tx);

        (
            (tokio::spawn(re_connect.0), re_connect.1),
            (tokio::spawn(disconnect_test.0), disconnect_test.1),
        )
    };

    // Need to connect first so Raft messages can be sent.
    loop_wait_connnect_to_peers_async(node_conn.clone(), expected_connected_addrs).await;

    // Send any requests here

    if let Some(amount_to_send) = amount_to_send {
        println!("Connect to user address: {:?}", peer_user_node.address);
        node.connect_to(peer_user_node.address).await.unwrap();
        node.send_address_request(peer_user_node.address, amount_to_send)
            .await
            .unwrap();
    }

    // REQUEST HANDLING
    let main_loop_handle = tokio::spawn({
        let mut node = node;
        let mut node_conn = node_conn;

        async move {
            node.send_startup_requests().await.unwrap();

            let mut exit = std::future::pending();
            while let Some(response) = node.handle_next_event(&mut exit).await {
                if node.handle_next_event_response(response).await == ResponseResult::Exit {
                    break;
                }
            }
            stop_re_connect_tx.send(()).unwrap();
            stop_disconnect_tx.send(()).unwrap();
            shutdown_connections(&mut node_conn).await;
        }
    });

    // Warp API
    let warp_handle = tokio::spawn({
        let (db, node, api_addr, api_tls) = api_inputs;

        println!("Warp API started on port {:?}", api_addr.port());
        println!();

        let mut bind_address = "0.0.0.0:0".parse::<SocketAddr>().unwrap();
        bind_address.set_port(api_addr.port());

        async move {
            use warp::Filter;
            warp::serve(
                routes::wallet_info(db.clone())
                    .or(routes::make_payment(db.clone(), node.clone()))
                    .or(routes::make_ip_payment(db.clone(), node.clone()))
                    .or(routes::request_donation(node.clone()))
                    .or(routes::wallet_keypairs(db.clone()))
                    .or(routes::import_keypairs(db.clone()))
                    .or(routes::update_running_total(node.clone()))
                    .or(routes::wallet_encapsulation_data(db.clone()))
                    .or(routes::payment_address(db)),
            )
            .tls()
            .key(&api_tls.pem_rsa_private_keys)
            .cert(&api_tls.pem_certs)
            .run(bind_address)
            .await;
        }
    });

    let (main_result, warp_result, conn, disconn) = tokio::join!(
        main_loop_handle,
        warp_handle,
        conn_loop_handle,
        disconn_loop_handle
    );
    main_result.unwrap();
    warp_result.unwrap();
    conn.unwrap();
    disconn.unwrap();
}

fn clap_app<'a, 'b>() -> App<'a, 'b> {
    App::new("Zenotta Mining Node")
        .about("Runs a basic miner node.")
        .arg(
            Arg::with_name("config")
                .long("config")
                .short("c")
                .help("Run the user node using the given config file.")
                .takes_value(true),
        )
        .arg(
            Arg::with_name("tls_config")
                .long("tls_config")
                .help("Use file to provide tls configuration options.")
                .takes_value(true),
        )
        .arg(
            Arg::with_name("initial_block_config")
                .long("initial_block_config")
                .help("Run the compute node using the given initial block config file.")
                .takes_value(true),
        )
        .arg(
            Arg::with_name("api_port")
                .long("api_port")
                .help("The port to run the http API from")
                .takes_value(true),
        )
        .arg(
            Arg::with_name("amount")
                .short("a")
                .long("amount")
                .help("The amount of tokens to send to a recipient address")
                .takes_value(true),
        )
        .arg(
            Arg::with_name("auto_donate")
                .long("auto_donate")
                .help("The amount of tokens to send any requester")
                .takes_value(true),
        )
        .arg(
            Arg::with_name("index")
                .short("i")
                .long("index")
                .help("Run the specified user node index from config file")
                .takes_value(true),
        )
        .arg(
            Arg::with_name("compute_index")
                .long("compute_index")
                .help("Endpoint index of a compute node that the user should connect to")
                .takes_value(true),
        )
        .arg(
            Arg::with_name("peer_user_index")
                .long("peer_user_index")
                .help("Endpoint index of a peer user node that the user should connect to")
                .takes_value(true),
        )
        .arg(
            Arg::with_name("passphrase")
                .long("passphrase")
                .help("Enter a password or passphase for the encryption of the Wallet.")
                .takes_value(true),
        )
}

fn load_settings(matches: &clap::ArgMatches) -> config::Config {
    let mut settings = config::Config::default();
    let setting_file = matches
        .value_of("config")
        .unwrap_or("src/bin/node_settings.toml");
    let intial_block_setting_file = matches
        .value_of("initial_block_config")
        .unwrap_or("src/bin/initial_block.json");
    let tls_setting_file = matches
        .value_of("tls_config")
        .unwrap_or("src/bin/tls_certificates.json");

    settings.set_default("user_api_port", 3000).unwrap();
    settings.set_default("user_node_idx", 0).unwrap();
    settings.set_default("user_compute_node_idx", 0).unwrap();
    settings.set_default("peer_user_node_idx", 0).unwrap();
    settings.set_default("user_setup_tx_max_count", 0).unwrap();
    settings.set_default("user_auto_donate", 0).unwrap();

    settings
        .merge(config::File::with_name(setting_file))
        .unwrap();
    settings
        .merge(config::File::with_name(intial_block_setting_file))
        .unwrap();
    settings
        .merge(config::File::with_name(tls_setting_file))
        .unwrap();

    if let Some(index) = matches.value_of("index") {
        settings.set("user_node_idx", index).unwrap();
        let mut db_mode = settings.get_table("user_db_mode").unwrap();
        if let Some(test_idx) = db_mode.get_mut("Test") {
            let index = index.parse::<usize>().unwrap();
            let index = index + test_idx.clone().try_into::<usize>().unwrap();
            *test_idx = config::Value::new(None, index.to_string());
            settings.set("user_db_mode", db_mode).unwrap();
        }
    }

    if let Some(api_port) = matches.value_of("api_port") {
        settings.set("user_api_port", api_port).unwrap();
    }

    if let Some(index) = matches.value_of("compute_index") {
        settings.set("user_compute_node_idx", index).unwrap();
    }

    if let Some(index) = matches.value_of("peer_user_index") {
        settings.set("peer_user_node_idx", index).unwrap();
    }

    if let Some(index) = matches.value_of("passphrase") {
        settings.set("passphrase", index).unwrap();
    }

    if let Some(api_port) = matches.value_of("auto_donate") {
        settings.set("user_auto_donate", api_port).unwrap();
    }

    settings
}

fn configuration(settings: config::Config) -> UserNodeConfig {
    settings.try_into().unwrap()
}

#[cfg(test)]
mod test {
    use super::*;
    use system::configurations::DbMode;

    #[test]
    fn validate_startup_no_args() {
        let args = vec!["bin_name"];
        let expected = DbMode::Test(1000);

        validate_startup_common(args, expected);
    }

    #[test]
    fn validate_startup_raft_1() {
        let args = vec![
            "bin_name",
            "--config=src/bin/node_settings_local_raft_1.toml",
        ];
        let expected = DbMode::Test(1000);

        validate_startup_common(args, expected);
    }

    #[test]
    fn validate_startup_raft_2_index_1() {
        let args = vec![
            "bin_name",
            "--config=src/bin/node_settings_local_raft_2.toml",
            "--index=1",
        ];
        let expected = DbMode::Test(1001);

        validate_startup_common(args, expected);
    }

    #[test]
    fn validate_startup_raft_3() {
        let args = vec![
            "bin_name",
            "--config=src/bin/node_settings_local_raft_1.toml",
        ];
        let expected = DbMode::Test(1000);

        validate_startup_common(args, expected);
    }

    fn validate_startup_common(args: Vec<&str>, expected: DbMode) {
        //
        // Act
        //
        let app = clap_app();
        let matches = app.get_matches_from_safe(args.into_iter()).unwrap();
        let settings = load_settings(&matches);
        let config = configuration(settings);

        //
        // Assert
        //
        assert_eq!(config.user_db_mode, expected);
    }
}
