# Run Loom concurrency tests
# Loom performs exhaustive testing of all possible thread interleavings
# to detect data races, deadlocks, and memory ordering issues.

Write-Host "Running Loom concurrency tests..." -ForegroundColor Cyan
Write-Host "This will exhaustively test all thread interleavings." -ForegroundColor Yellow
Write-Host ""

$env:RUSTFLAGS = "--cfg loom"

try {
    cargo test --test loom_tests --release --features loom -- --nocapture
    
    if ($LASTEXITCODE -eq 0) {
        Write-Host ""
        Write-Host "✓ All Loom tests passed!" -ForegroundColor Green
        Write-Host "The library has been verified for concurrency correctness." -ForegroundColor Green
    } else {
        Write-Host ""
        Write-Host "✗ Loom tests failed!" -ForegroundColor Red
        exit $LASTEXITCODE
    }
} finally {
    # Clean up environment variable
    $env:RUSTFLAGS = ""
}
