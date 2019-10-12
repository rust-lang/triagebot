#!/bin/bash
set -euo pipefail
IFS=$'\n\t'

ci/check-nightly-version.sh
cargo test
docker build -t rust-triagebot .
