//! App to run a mining node.

use clap::{App, Arg};
use std::collections::BTreeSet;
use system::configurations::DbMode;
use system::upgrade::{
    dump_db, get_db_to_dump_no_checks, get_upgrade_compute_db, get_upgrade_storage_db,
    get_upgrade_wallet_db, upgrade_compute_db, upgrade_storage_db, upgrade_wallet_db, DbCfg,
    DbSpecInfo, UpgradeCfg, UpgradeError, DB_SPEC_INFOS,
};

const NODE_TYPES: &[&str] = &["compute", "storage", "user", "miner"];

#[derive(PartialEq, Eq, Clone, Copy, Debug)]
enum Processing {
    Read,
    Upgrade,
}

#[tokio::main]
async fn main() -> Result<(), UpgradeError> {
    tracing_subscriber::fmt::init();

    let matches = clap_app().get_matches();
    let (processing, db_modes, upgrade_cfg) = configuration(load_settings(&matches), &matches);

    match processing {
        Processing::Read => {
            if let Err(e) = process_read(db_modes) {
                println!("Read out error, aborting: {:?}", e);
                return Err(e);
            }
        }
        Processing::Upgrade => {
            if let Err(e) = process_upgrade(db_modes, upgrade_cfg) {
                println!("Upgrade error, aborting: {:?}", e);
                return Err(e);
            }
        }
    }

    Ok(())
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
fn process_upgrade(
    db_modes: Vec<(String, DbMode)>,
    upgrade_cfg: UpgradeCfg,
) -> Result<(), UpgradeError> {
    println!("Upgrade with config {:?}", db_modes);
    for (node_type, mode) in db_modes {
        println!("Upgrade Database {}, {:?}", node_type, mode);
        let extra = Default::default();
        match node_type.as_str() {
            "compute" => upgrade_compute_db(get_upgrade_compute_db(mode, extra)?, &upgrade_cfg)?,
            "storage" => upgrade_storage_db(get_upgrade_storage_db(mode, extra)?, &upgrade_cfg)?,
            "user" => upgrade_wallet_db(get_upgrade_wallet_db(mode, extra)?, &upgrade_cfg)?,
            "miner" => upgrade_wallet_db(get_upgrade_wallet_db(mode, extra)?, &upgrade_cfg)?,
            _ => return Err(UpgradeError::ConfigError("Type does not exists")),
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

fn clap_app<'a, 'b>() -> App<'a, 'b> {
    App::new("Zenotta Database Upgrade")
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
                .help("Run the upgrade for type (all or compute, storage, user, miner)")
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
        .arg(
            Arg::with_name("passphrase")
                .long("passphrase")
                .help("Enter a password or passphase for the encryption of the Wallet.")
                .takes_value(true),
        )
        .arg(
            Arg::with_name("compute_block")
                .long("compute_block")
                .help("Specify what to do with compute node: mine or discard")
                .takes_value(true),
        )
        .arg(
            Arg::with_name("ignore")
                .long("ignore")
                .help("Ignore some toml nodes: ignore=compute.0,storage.0,user.1,miner.1")
                .takes_value(true),
        )
}

fn load_settings(matches: &clap::ArgMatches) -> config::Config {
    let mut settings = config::Config::default();
    let setting_file = matches
        .value_of("config")
        .unwrap_or("src/bin/node_settings.toml");

    settings
        .merge(config::File::with_name(setting_file))
        .unwrap();

    settings
}

fn configuration(
    settings: config::Config,
    matches: &clap::ArgMatches,
) -> (Processing, Vec<(String, DbMode)>, UpgradeCfg) {
    let passphrase = matches
        .value_of("passphrase")
        .unwrap_or_default()
        .to_owned();
    let node_type = matches.value_of("type").unwrap();
    let processing = match matches.value_of("processing").unwrap() {
        "read" => Processing::Read,
        "upgrade" => Processing::Upgrade,
        v => panic!("expect processing to be read or upgrade: {}", v),
    };
    let db_cfg = match matches.value_of("compute_block").unwrap() {
        "mine" => DbCfg::ComputeBlockToMine,
        "discard" => DbCfg::ComputeBlockInStorage,
        v => panic!("expect compute_block to be miner or discard: {}", v),
    };
    let raft_len = settings.get_array("storage_nodes").unwrap().len();
    let upgrade_cfg = UpgradeCfg {
        raft_len,
        passphrase,
        db_cfg,
    };

    let ignore = matches.value_of("ignore").unwrap_or("");
    let ignore: BTreeSet<String> = ignore.split(',').map(|v| v.to_owned()).collect();

    let db_modes = if node_type == "all" {
        let mut upgrades = Vec::new();
        for node_type in NODE_TYPES {
            let db_mode_name = format!("{}_db_mode", node_type);
            let node_specs_name = format!("{}_nodes", node_type);
            let node_specs = settings.get_array(&node_specs_name).unwrap();
            for node_index in 0..node_specs.len() {
                if !ignore.contains(&format!("{}.{}", node_type, node_index)) {
                    if let DbMode::Test(index) = settings.get(&db_mode_name).unwrap() {
                        let db_mode = DbMode::Test(index + node_index);
                        upgrades.push((node_type.to_string(), db_mode));
                    }
                }
            }
        }
        upgrades
    } else if NODE_TYPES.contains(&node_type) {
        let db_mode_name = format!("{}_db_mode", node_type);
        let db_mode: DbMode = settings.get(&db_mode_name).unwrap();
        let db_mode = if let DbMode::Test(index) = &db_mode {
            let node_index = matches.value_of("index").unwrap_or("0");
            let node_index = node_index.parse::<usize>().unwrap();
            DbMode::Test(index + node_index)
        } else {
            db_mode
        };
        vec![(node_type.to_string(), db_mode)]
    } else {
        panic!("type must be one of all or {}", NODE_TYPES.join(", "));
    };

    (processing, db_modes, upgrade_cfg)
}

#[cfg(test)]
mod test {
    use super::*;
    use system::configurations::DbMode;

    #[test]
    fn validate_startup_read_all_raft_1() {
        let args = vec![
            "bin_name",
            "--config=src/bin/node_settings_local_raft_1.toml",
            "--processing=read",
            "--type=all",
            "--compute_block=mine",
        ];
        let expected = (
            Processing::Read,
            vec![
                ("compute".to_owned(), DbMode::Test(0)),
                ("storage".to_owned(), DbMode::Test(0)),
                ("user".to_owned(), DbMode::Test(1000)),
                ("miner".to_owned(), DbMode::Test(0)),
            ],
            UpgradeCfg {
                raft_len: 1,
                passphrase: String::new(),
                db_cfg: DbCfg::ComputeBlockToMine,
            },
        );

        validate_startup_common(args, expected);
    }

    #[test]
    fn validate_startup_upgrade_user_raft_1() {
        let args = vec![
            "bin_name",
            "--config=src/bin/node_settings_local_raft_1.toml",
            "--processing=upgrade",
            "--index=1",
            "--type=user",
            "--passphrase=TestPassPhrase",
            "--compute_block=discard",
        ];
        let expected = (
            Processing::Upgrade,
            vec![("user".to_owned(), DbMode::Test(1001))],
            UpgradeCfg {
                raft_len: 1,
                passphrase: "TestPassPhrase".to_owned(),
                db_cfg: DbCfg::ComputeBlockInStorage,
            },
        );

        validate_startup_common(args, expected);
    }

    #[test]
    fn validate_startup_read_all_raft_3() {
        let args = vec![
            "bin_name",
            "--config=src/bin/node_settings_local_raft_3.toml",
            "--processing=read",
            "--type=compute",
            "--compute_block=mine",
        ];
        let expected = (
            Processing::Read,
            vec![("compute".to_owned(), DbMode::Test(0))],
            UpgradeCfg {
                raft_len: 3,
                passphrase: String::new(),
                db_cfg: DbCfg::ComputeBlockToMine,
            },
        );

        validate_startup_common(args, expected);
    }

    #[test]
    fn validate_startup_read_all_raft_2() {
        let args = vec![
            "bin_name",
            "--config=src/bin/node_settings_local_raft_2.toml",
            "--processing=read",
            "--type=all",
            "--compute_block=mine",
            "--ignore=miner.1,miner.2,miner.3,miner.4,miner.5,miner.6,user.1",
        ];
        let expected = (
            Processing::Read,
            vec![
                ("compute".to_owned(), DbMode::Test(0)),
                ("compute".to_owned(), DbMode::Test(1)),
                ("storage".to_owned(), DbMode::Test(0)),
                ("storage".to_owned(), DbMode::Test(1)),
                ("user".to_owned(), DbMode::Test(1000)),
                ("miner".to_owned(), DbMode::Test(0)),
            ],
            UpgradeCfg {
                raft_len: 2,
                passphrase: String::new(),
                db_cfg: DbCfg::ComputeBlockToMine,
            },
        );

        validate_startup_common(args, expected);
    }

    fn validate_startup_common(
        args: Vec<&str>,
        expected: (Processing, Vec<(String, DbMode)>, UpgradeCfg),
    ) {
        //
        // Act
        //
        let app = clap_app();
        let matches = app.get_matches_from_safe(args.into_iter()).unwrap();
        let settings = load_settings(&matches);
        let config = configuration(settings, &matches);

        //
        // Assert
        //
        assert_eq!(config, expected);
    }
}
