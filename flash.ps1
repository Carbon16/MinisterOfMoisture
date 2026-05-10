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

if ($Monitor.IsPresent) {
    # If the user wants to monitor, just invoke `cargo run`.
    # `.cargo/config.toml` defines `runner = "espflash flash --monitor"`
    Write-Host "Running: cargo run"
    if ($Release.IsPresent) {
        cargo run --release
    } else {
        cargo run
    }
} else {
    # If not monitoring, use the known output path in `/tmp/m/xtensa-esp32-espidf`
    if ($Release.IsPresent) {
        $image = "/tmp/m/xtensa-esp32-espidf/release/MinisterOfMoisture"
    } else {
        $image = "/tmp/m/xtensa-esp32-espidf/debug/MinisterOfMoisture"
    }
    Write-Host "Running: espflash flash $image"
    $flashArgs = @('flash', '--port', $Port, '--baud', "$FlashBaud")
    & espflash @flashArgs $image
}

$flashExit = $LASTEXITCODE
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
