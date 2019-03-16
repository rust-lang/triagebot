#!/bin/bash
set -euo pipefail
IFS=$'\n\t'

rustup component add rustfmt clippy

ci/check-nightly-version.sh
cargo test
cargo fmt -- --check
cargo clippy -- -Dwarnings

docker build -t rust-triagebot .
