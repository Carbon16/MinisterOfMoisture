# MinisterOfMoisture

This project uses the `esp-idf-hal` and `mfrc522` crates to communicate with an RFID module on an ESP32.

## Known Compilation Issues and Fixes

### 1. `esp-idf-sys` Build Script Failure (`pkg_resources` missing)
If you encounter a build script failure for `esp-idf-sys` reporting `pkg_resources cannot be imported`, this is due to an incompatibility between ESP-IDF v5.1's build system and Python 3.12+ / newer versions of `setuptools`.

**Fix:**
Manually install the correct legacy dependencies into the Espressif Python environment:
```powershell
# Activate or locate your IDF python environment (typically in .espressif/python_env/...)
# Run the following using the exact python.exe from that env:

# 1. Downgrade setuptools because version 70.0+ removes the `pkg_resources` API entirely
python -m pip install "setuptools<70"

# 2. Downgrade ruamel.yaml so older pkg_resources can correctly detect its metadata
python -m pip install "ruamel.yaml<0.18"

# 3. Ensure all ESP-IDF core requirements are installed
python -m pip install -r "C:\Users\Student\.espressif\esp-idf\v5.1\tools\requirements\requirements.core.txt"
```

### 2. `esp-idf-hal` 0.43 / `mfrc522` 0.8 SPI Configuration changes
The latest versions of these crates migrated to the `embedded-hal` v1.0 standard, requiring initialization changes:

1. **`SpiDeviceDriver` setup**: Use `spi::SpiDeviceDriver::new_single()` instead of `new()`, and pass `spi::config::MODE_0` (which delegates to the newer embedded-hal mode struct) instead of the legacy `embedded_hal::spi::MODE_0`.
2. **`MFRC522` initialization**: The `mfrc522` 0.8.0 crate expects an explicit communication wrapper. You must wrap the `spi_device` inside `mfrc522::comm::blocking::spi::SpiInterface::new()`.
3. **MFRC522 state machine**: `Mfrc522::new()` now returns an `Uninitialized` state object and does not return a `Result`. You must chain `.init()` before calling `.expect()`.

Example working configuration:
```rust
let spi_config = spi::config::Config::new()
    .baudrate(1.MHz().into())
    .data_mode(spi::config::MODE_0);

let spi_device = spi::SpiDeviceDriver::new_single(
    spi,
    sclk,
    serial_out, // SDO
    Some(serial_in), // SDI
    Some(cs), // CS
    &spi::config::DriverConfig::new(),
    &spi_config,
).expect("Failed to create SPI device");

let spi_itf = mfrc522::comm::blocking::spi::SpiInterface::new(spi_device);
let mut mfrc522 = Mfrc522::new(spi_itf).init().expect("Failed to initialize MFRC522");
```

## Flashing
To build, flash, and view the serial monitor, simply run:
```powershell
cargo run
```
