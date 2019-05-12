#!/bin/bash
set -euo pipefail
IFS=$'\n\t'

rustup component add rustfmt 

ci/check-nightly-version.sh
cargo test
cargo fmt -- --check

docker build -t rust-triagebot .
