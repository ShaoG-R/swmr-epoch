#!/bin/bash
# Run Loom concurrency tests
# Loom performs exhaustive testing of all possible thread interleavings
# to detect data races, deadlocks, and memory ordering issues.

echo -e "\033[36mRunning Loom concurrency tests...\033[0m"
echo -e "\033[33mThis will exhaustively test all thread interleavings.\033[0m"
echo ""

export RUSTFLAGS="--cfg loom"

cargo test --test loom_tests --release --features loom -- --nocapture

if [ $? -eq 0 ]; then
    echo ""
    echo -e "\033[32m✓ All Loom tests passed!\033[0m"
    echo -e "\033[32mThe library has been verified for concurrency correctness.\033[0m"
else
    echo ""
    echo -e "\033[31m✗ Loom tests failed!\033[0m"
    exit 1
fi
