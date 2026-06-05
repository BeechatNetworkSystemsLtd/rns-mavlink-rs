#!/usr/bin/env bash

set -e
set -x

. /opt/yocto-sdk/environment-setup-cortexa7t2hf-neon-vfpv4-ostl-linux-gnueabi
rustup target add armv7-unknown-linux-gnueabihf
cargo build --release --target "armv7-unknown-linux-gnueabihf"
cargo build --release --target "armv7-unknown-linux-gnueabihf" --bin fc
cargo build --release --target "armv7-unknown-linux-gnueabihf" --bin gc
./create-ota.sh

exit 0
