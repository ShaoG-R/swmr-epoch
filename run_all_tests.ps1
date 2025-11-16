# Run all tests: unit tests, integration tests, doc tests, and Loom tests

Write-Host "Running complete test suite..." -ForegroundColor Cyan
Write-Host ""

# 1. Run unit tests
Write-Host "[1/3] Running unit tests and integration tests..." -ForegroundColor Yellow
cargo test --release
if ($LASTEXITCODE -ne 0) {
    Write-Host "✗ Unit tests failed!" -ForegroundColor Red
    exit $LASTEXITCODE
}
Write-Host "✓ Unit and integration tests passed" -ForegroundColor Green
Write-Host ""

# 2. Run benchmarks in test mode (don't actually benchmark, just verify they compile and run)
Write-Host "[2/3] Verifying benchmarks..." -ForegroundColor Yellow
cargo test --benches --release
if ($LASTEXITCODE -ne 0) {
    Write-Host "✗ Benchmark verification failed!" -ForegroundColor Red
    exit $LASTEXITCODE
}
Write-Host "✓ Benchmarks verified" -ForegroundColor Green
Write-Host ""

# 3. Run Loom concurrency tests
Write-Host "[3/3] Running Loom concurrency tests..." -ForegroundColor Yellow
$env:RUSTFLAGS = "--cfg loom"
try {
    cargo test --test loom_tests --release --features loom -- --nocapture
    if ($LASTEXITCODE -ne 0) {
        Write-Host "✗ Loom tests failed!" -ForegroundColor Red
        exit $LASTEXITCODE
    }
    Write-Host "✓ Loom concurrency tests passed" -ForegroundColor Green
} finally {
    $env:RUSTFLAGS = ""
}

Write-Host ""
Write-Host "════════════════════════════════════════" -ForegroundColor Cyan
Write-Host "✓ ALL TESTS PASSED!" -ForegroundColor Green
Write-Host "════════════════════════════════════════" -ForegroundColor Cyan
Write-Host ""
Write-Host "Summary:" -ForegroundColor White
Write-Host "  • Unit tests: PASSED" -ForegroundColor Green
Write-Host "  • Integration tests: PASSED" -ForegroundColor Green
Write-Host "  • Doc tests: PASSED" -ForegroundColor Green
Write-Host "  • Benchmarks: VERIFIED" -ForegroundColor Green
Write-Host "  • Loom concurrency tests: PASSED" -ForegroundColor Green
Write-Host ""
