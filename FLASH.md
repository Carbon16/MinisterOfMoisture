# Flashing the ESP32

This repository includes `flash.ps1`, a PowerShell helper that builds and flashes the project using `espflash`.

Prerequisites
- Install Rust toolchain and ensure `cargo` is on PATH.
- Install `espflash` (one of):
  - `cargo install espflash`
  - or `pip install espflash`

Usage
- Open PowerShell in the project root and run:

```powershell
Get-PnpDevice -Class Ports -PresentOnly | Select-Object Name
.\flash.ps1 -Port COM3 -Release
```

- To flash and immediately open a serial monitor (default 115200 baud):

```powershell
.\flash.ps1 -Port COM3 -Release -Monitor
```

- To override flash or monitor baud rates:

```powershell
.\flash.ps1 -Port COM3 -FlashBaud 460800 -Monitor -MonitorBaud 115200
```

Notes
- The script prefers `espflash` which will build the Cargo project and flash the ESP32. If `espflash` is not found it prints install instructions.
- Adjust the `-Port` value (e.g. `COM3`) to match your device.
 