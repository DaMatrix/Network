#!/bin/sh

echo " "
echo "//-----------------------------//"
echo "Building nodes"
echo "//-----------------------------//"
echo " "
cargo build --bins --release
echo " "
echo "//-----------------------------//"
echo "Delete databases"
echo "//-----------------------------//"
echo " "
rm -rf src/db/db/test.* src/wallet/wallet/test.*
echo " "
echo "//-----------------------------//"
echo "Running nodes for node_settings_local_raft.toml"
echo "//-----------------------------//"
echo " "
RUST_LOG="debug,raft=warn" target/release/storage --config=src/bin/node_settings_local_raft.toml --index=1 > storage_1.log 2>&1 &
s1=$!
RUST_LOG="debug,raft=warn" target/release/storage --config=src/bin/node_settings_local_raft.toml > storage_0.log 2>&1 &
s0=$!
RUST_LOG="warn" target/release/compute --config=src/bin/node_settings_local_raft.toml --index=1 > compute_1.log 2>&1 &
c1=$!
RUST_LOG="warn" target/release/compute --config=src/bin/node_settings_local_raft.toml > compute_0.log 2>&1 &
c0=$!
RUST_LOG="warn" target/release/miner --config=src/bin/node_settings_local_raft.toml  --index=5 --compute_index=1 --compute_connect > miner_5.log 2>&1 &
m5=$!
RUST_LOG="warn" target/release/miner --config=src/bin/node_settings_local_raft.toml  --index=4 --compute_index=0 --compute_connect > miner_4.log 2>&1 &
m4=$!
RUST_LOG="warn" target/release/miner --config=src/bin/node_settings_local_raft.toml  --index=3 --compute_index=1 --compute_connect > miner_3.log 2>&1 &
m3=$!
RUST_LOG="warn" target/release/miner --config=src/bin/node_settings_local_raft.toml  --index=2 --compute_index=0 --compute_connect > miner_2.log 2>&1 &
m2=$!
RUST_LOG="warn" target/release/miner --config=src/bin/node_settings_local_raft.toml  --index=1 --compute_index=1 --compute_connect > miner_1.log 2>&1 &
m1=$!
RUST_LOG="warn" target/release/miner --config=src/bin/node_settings_local_raft.toml  --compute_connect > miner_0.log 2>&1 &
m0=$!
RUST_LOG="debug" target/release/user  --config=src/bin/node_settings_local_raft.toml --compute_connect > user_0.log 2>&1 &
u0=$!

echo $s1 $s0 $c1 $c0 $m5 $m4 $m3 $m2 $m1 $m0 $u0
trap 'echo Kill All $s1 $s0 $c1 $c0 $m5 $m4 $m3 $m2 $m1 $m0 $u0; kill $s1 $s0 $c1 $c0 $m5 $m4 $m3 $m2 $m1 $m0 $u0' INT
tail -f storage_1.log