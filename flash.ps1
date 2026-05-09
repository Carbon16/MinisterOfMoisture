Param(
    [string]$Port = "COM6",
    [int]$FlashBaud = 460800,
    [int]$MonitorBaud = 115200,
    [switch]$Release,
    [switch]$Monitor
)

Write-Host "Flash script for ESP32 (RC522 project)"
Write-Host "Port: $Port; FlashBaud: $FlashBaud; Monitor: $Monitor; MonitorBaud: $MonitorBaud; Release: $Release"

# Prefer using espflash (it will build the Cargo project and flash the chip)
$espflash = Get-Command espflash -ErrorAction SilentlyContinue
if (-not $espflash) {
    Write-Host "espflash not found." -ForegroundColor Yellow
    Write-Host "Install it with one of the following commands:" -ForegroundColor Yellow
    Write-Host "  cargo install espflash" -ForegroundColor Green
    Write-Host "  or" -ForegroundColor Green
    Write-Host "  pip install espflash" -ForegroundColor Green
    Write-Host "After installing, re-run this script: .\\flash.ps1 -Port COM3 -Release -Monitor" -ForegroundColor Cyan
    exit 1
}

# Build and flash
if ($Release.IsPresent) {
    Write-Host "Running: cargo build --release"
    cargo build --release
} else {
    Write-Host "Running: cargo build"
    cargo build
}

if ($LASTEXITCODE -ne 0) {
    Write-Host "cargo build failed (exit $LASTEXITCODE)" -ForegroundColor Red
    exit $LASTEXITCODE
}

# Try to locate the built firmware image under target/ (bin/elf/bin-like files)
$projectRoot = Get-Location
$cargoToml = Join-Path $projectRoot "Cargo.toml"
$pkgName = $null
if (Test-Path $cargoToml) {
    $content = Get-Content $cargoToml -Raw
    if ($content -match 'name\s*=\s*"([^"]+)"') { $pkgName = $matches[1] }
}

$image = $null
if ($pkgName) {
    # search for files that match package name (common outputs: .bin, .elf, no-ext)
    $candidates = Get-ChildItem -Path (Join-Path $projectRoot 'target') -Recurse -ErrorAction SilentlyContinue |
                  Where-Object { $_.Name -like "$pkgName*" -and -not $_.PSIsContainer } |
                  Sort-Object LastWriteTime -Descending
    if ($candidates.Count -gt 0) { $image = $candidates[0].FullName }
}

if (-not $image) {
    Write-Host "Could not find built image automatically; falling back to letting espflash build (use -Release to build manually)" -ForegroundColor Yellow
    $flashArgs = @('flash', '--port', $Port, '--baud', "$FlashBaud", '.')
    Write-Host "Running: espflash $($flashArgs -join ' ')"
    & espflash @flashArgs
    $flashExit = $LASTEXITCODE
} else {
    Write-Host "Found image: $image" -ForegroundColor Green
    $flashArgs = @('flash', '--port', $Port, '--baud', "$FlashBaud", $image)
    Write-Host "Running: espflash $($flashArgs -join ' ')"
    & espflash @flashArgs
    $flashExit = $LASTEXITCODE
}

if ($flashExit -ne 0) {
    Write-Host "Flashing failed with exit code $flashExit" -ForegroundColor Red
    exit $flashExit
}

# Optionally start serial monitor
if ($Monitor.IsPresent) {
    Write-Host "Starting serial monitor (press Ctrl+C to exit)..." -ForegroundColor Cyan
    try {
        $monitorArgs = @('monitor', '--port', $Port, '--baud', "$MonitorBaud")
        Write-Host "Running: espflash $($monitorArgs -join ' ')"
        & espflash @monitorArgs
        exit $LASTEXITCODE
    } catch {
        Write-Host "Failed to start monitor: $_" -ForegroundColor Red
        exit 1
    }
} else {
    Write-Host "Flash complete. Run with -Monitor to open serial output." -ForegroundColor Green
    exit 0
}
