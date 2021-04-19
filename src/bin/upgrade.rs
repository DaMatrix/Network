//! App to run a mining node.

use clap::{App, Arg};
use system::configurations::DbMode;
use system::upgrade::{
    dump_db, get_db_to_dump_no_checks, get_upgrade_compute_db, get_upgrade_storage_db,
    upgrade_compute_db, upgrade_storage_db, DbSpecInfo, UpgradeError, DB_SPEC_INFOS,
};

const NODE_TYPES: &[&str] = &["compute", "storage", "user", "miner"];

#[derive(PartialEq, Eq, Clone, Copy, Debug)]
enum Processing {
    Read,
    Upgrade,
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    let matches = App::new("Zenotta Database Upgrade")
        .about("Runs database upgrade.")
        .arg(
            Arg::with_name("config")
                .long("config")
                .short("c")
                .help("Run the upgrade using the given config file.")
                .takes_value(true),
        )
        .arg(
            Arg::with_name("index")
                .short("i")
                .long("index")
                .help("Run the upgrade for the specified node index from config file")
                .takes_value(true),
        )
        .arg(
            Arg::with_name("type")
                .long("type")
                .help("Run the upgrade for the given node type (all or compute, storage, user, miner)")
                .takes_value(true)
                .required(true),
        )
        .arg(
            Arg::with_name("processing")
                .long("processing")
                .help("Type of processing to do: read or upgrade")
                .takes_value(true)
                .required(true),
        )
        .get_matches();

    let (processing, db_modes) = {
        let mut settings = config::Config::default();
        let setting_file = matches
            .value_of("config")
            .unwrap_or("src/bin/node_settings.toml");

        settings
            .merge(config::File::with_name(setting_file))
            .unwrap();

        let node_type = matches.value_of("type").unwrap();
        let processing = match matches.value_of("processing").unwrap() {
            "read" => Processing::Read,
            "upgrade" => Processing::Upgrade,
            _ => panic!("expect processing to be read or upgrade"),
        };

        let db_modes = if node_type == "all" {
            let mut upgrades = Vec::new();
            for node_type in NODE_TYPES {
                let db_mode_name = format!("{}_db_mode", node_type);
                let node_specs_name = format!("{}_nodes", node_type);
                let node_specs = settings.get_array(&node_specs_name).unwrap();
                for node_index in 0..node_specs.len() {
                    if let DbMode::Test(index) = settings.get(&db_mode_name).unwrap() {
                        let db_mode = DbMode::Test(index + node_index);
                        upgrades.push((node_type.to_string(), db_mode));
                    }
                }
            }
            upgrades
        } else if NODE_TYPES.contains(&node_type) {
            let db_mode_name = format!("{}_db_mode", node_type);
            if let Some(index) = matches.value_of("index") {
                let mut db_mode = settings.get_table(&db_mode_name).unwrap();
                if let Some(test_idx) = db_mode.get_mut("Test") {
                    let index = index.parse::<usize>().unwrap();
                    let index = index + test_idx.clone().try_into::<usize>().unwrap();
                    *test_idx = config::Value::new(None, index.to_string());
                    settings.set(&db_mode_name, db_mode).unwrap();
                }
            }
            let db_mode: DbMode = settings.get(&db_mode_name).unwrap();
            vec![(node_type.to_string(), db_mode)]
        } else {
            panic!("type must be one of all or {}", NODE_TYPES.join(", "));
        };

        (processing, db_modes)
    };

    match processing {
        Processing::Read => {
            if let Err(e) = process_read(db_modes) {
                println!("Read out error, aborting: {:?}", e);
            }
        }
        Processing::Upgrade => {
            if let Err(e) = process_upgrade(db_modes) {
                println!("Upgrade error, aborting: {:?}", e);
            }
        }
    }
}

/// Process reading databases, format in a rust ready constants.
fn process_read(db_modes: Vec<(String, DbMode)>) -> Result<(), UpgradeError> {
    println!("/// !!! AUTOGENERATED: DO NOT EDIT !!!");
    println!("/// Generated with: `path_to_upgrade_bin/upgrade --type all --processing read > path_to_file.rs`");
    println!("///");
    println!("/// Upgrade with config {:?}", db_modes);
    println!("/// Preserved hard coded compute database");
    println!("pub type DbEntryType = (&'static [u8], &'static [u8], &'static [u8]);");
    println!();
    for (node_type, mode) in db_modes {
        for spec in DB_SPEC_INFOS.iter().filter(|s| s.node_type == node_type) {
            let raft_name = raft_for_spec(spec);
            println!("/// Database for {}{}, {:?}", node_type, raft_name, mode);

            let name = format!("{}{}_DB_V0_2_0", spec.node_type, raft_name).to_ascii_uppercase();
            println!("pub const {}: &[DbEntryType] = &[", name);

            let db = get_db_to_dump_no_checks(mode, spec, None)?;
            for column_key_value in dump_db(&db) {
                println!("({}),", column_key_value);
            }
            println!("];");
        }
    }
    Ok(())
}

/// Process reading databases, format in a rust ready constants.
fn process_upgrade(db_modes: Vec<(String, DbMode)>) -> Result<(), UpgradeError> {
    println!("Upgrade with config {:?}", db_modes);
    for (node_type, mode) in db_modes {
        println!("Upgrade Database {}, {:?}", node_type, mode);
        match node_type.as_str() {
            "compute" => upgrade_compute_db(get_upgrade_compute_db(mode, None)?)?,
            "storage" => upgrade_storage_db(get_upgrade_storage_db(mode, None)?)?,
            _ => return Err(UpgradeError::ConfigError("Not implemented for this type")),
        };
        println!("Done Upgrade Database {}, {:?}", node_type, mode);
    }
    Ok(())
}

/// Get the raft part of the name depending on the spec
fn raft_for_spec(spec: &DbSpecInfo) -> &str {
    if spec.suffix.contains("raft") {
        "_raft"
    } else {
        ""
    }
}
