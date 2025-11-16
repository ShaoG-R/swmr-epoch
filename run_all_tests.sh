#!/bin/bash
# Run all tests: unit tests, integration tests, doc tests, and Loom tests

set -e  # Exit on first error

echo -e "\033[36mRunning complete test suite...\033[0m"
echo ""

# 1. Run unit tests
echo -e "\033[33m[1/3] Running unit tests and integration tests...\033[0m"
cargo test --release
echo -e "\033[32m✓ Unit and integration tests passed\033[0m"
echo ""

# 2. Run benchmarks in test mode
echo -e "\033[33m[2/3] Verifying benchmarks...\033[0m"
cargo test --benches --release
echo -e "\033[32m✓ Benchmarks verified\033[0m"
echo ""

# 3. Run Loom concurrency tests
echo -e "\033[33m[3/3] Running Loom concurrency tests...\033[0m"
export RUSTFLAGS="--cfg loom"
cargo test --test loom_tests --release --features loom -- --nocapture
echo -e "\033[32m✓ Loom concurrency tests passed\033[0m"
unset RUSTFLAGS

echo ""
echo -e "\033[36m════════════════════════════════════════\033[0m"
echo -e "\033[32m✓ ALL TESTS PASSED!\033[0m"
echo -e "\033[36m════════════════════════════════════════\033[0m"
echo ""
echo -e "\033[37mSummary:\033[0m"
echo -e "  \033[32m• Unit tests: PASSED\033[0m"
echo -e "  \033[32m• Integration tests: PASSED\033[0m"
echo -e "  \033[32m• Doc tests: PASSED\033[0m"
echo -e "  \033[32m• Benchmarks: VERIFIED\033[0m"
echo -e "  \033[32m• Loom concurrency tests: PASSED\033[0m"
echo ""
