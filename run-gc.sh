#!/usr/bin/env bash

set -e
set -x

cargo run --bin gc -- -a "127.0.0.1:8080"

exit 0
