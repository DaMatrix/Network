//! App to run a compute node.

use clap::{App, Arg};
use system::{ComputeNode, Response};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();

    let matches = App::new("Zenotta Compute Node")
        .about("Runs a basic compute node.")
        .arg(
            Arg::with_name("ip")
                .long("ip")
                .value_name("ADDRESS")
                .help("Run the compute node at the given IP address (defaults to 0.0.0.0)")
                .takes_value(true),
        )
        .arg(
            Arg::with_name("port")
                .short("p")
                .long("port")
                .help("Run the compute node at the given port number (defaults to 0)")
                .takes_value(true),
        )
        .get_matches();

    let endpoint = format!(
        "{}:{}",
        matches.value_of("ip").unwrap_or("0.0.0.0"),
        matches.value_of("port").unwrap_or("0")
    );

    let node = ComputeNode::new(endpoint.parse().unwrap()).await?;

    println!("Started node at {}", node.address());

    // REQUEST HANDLING
    tokio::spawn({
        let mut node = node.clone();

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
                        let _flood = node.flood_rand_num_to_requesters().await.unwrap();
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
