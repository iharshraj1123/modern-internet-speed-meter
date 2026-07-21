# Internet Speed Meter & Telemetry Dashboard

[![Latest Release](https://img.shields.io/github/v/release/iharshraj1123/modern-internet-speed-meter?color=10b981&label=Release&style=flat-square)](https://github.com/iharshraj1123/modern-internet-speed-meter/releases/tag/v1.1.0)
[![License](https://img.shields.io/badge/License-NC--SA%20%2B%20Attribution-violet?style=flat-square)](LICENSE)
[![Platform](https://img.shields.io/badge/Platform-Windows%20x64-sky?style=flat-square)](https://github.com/iharshraj1123/modern-internet-speed-meter/releases)

A glassmorphic desktop widget and network telemetry dashboard built with Tauri v2, Rust, and SvelteKit. It monitors network transfer rates, tracks per-process bandwidth usage, logs telemetry data locally to SQLite, and provides customizable desktop overlay graphs.

---

## Features

### Floating Desktop Overlay Widget
- Real-time download and upload speed monitoring with SVG graph rendering.
- Multiple graph layout options: Combined Dual Overlay, Stacked Separate Graphs, Download Only, Upload Only, or Hidden Mode.
- Vertical Auto-Collapse: In Hidden mode, the widget collapses to 34px height and disables vertical resizing while remembering expanded window dimensions.
- Customizable opacity, graph line styles (Dashed vs. Smooth Solid), and optional Peak Speed tag.
- Click-Through Mode and Position Locking.

### Telemetry Dashboard
- **Live View**: 60-second real-time network charts, active transfer speed averages (`Avg ↓` / `Avg ↑`), peak speed indicators (`Peak ↓` / `Peak ↑`), X-axis time scale markers, and active process bandwidth tables.
- **Process Analytics**: Real-time per-process network bandwidth tracking.
- **Historical Reports**: Daily, monthly, and hourly bandwidth usage telemetry stored in a local SQLite database.

### Customization & Interface
- 6 Accent Color Palettes (Emerald, Violet, Sky, Amber, Rose, Coral).
- System Default, Dark, and Light themes.
- Flexible measurement units (Decimal Bytes, Binary Bytes, Decimal Bits, Binary Bits).
- Categorized Appearance settings divided into Theme & Interface and Widget & Graphs sub-tabs.

### Power & Management
- Daily and Monthly Data Budget caps with progress bars.
- Customizable global hotkey support.
- System Tray integration with right-click menu.

---

## Download & Installation

Windows installers for Version 1.1.0 can be downloaded from the GitHub Releases page:

**[Download Version 1.1.0 Release](https://github.com/iharshraj1123/modern-internet-speed-meter/releases/tag/v1.1.0)**

Available formats:
- `Internet Speed Meter_1.1.0_x64-setup.exe` (Windows NSIS Setup Installer)
- `Internet Speed Meter_1.1.0_x64_en-US.msi` (Windows MSI Installer)

---

## Building Locally

### Prerequisites

1. **Node.js**: Install Node.js (LTS recommended).
2. **Rust**: Install Rust via [rustup](https://rustup.rs/).
3. **64-bit MinGW Compiler**: Rust's `x86_64-pc-windows-gnu` target requires a 64-bit MinGW toolchain on Windows.

### Build Steps

1. Clone the repository:
   ```bash
   git clone https://github.com/iharshraj1123/modern-internet-speed-meter.git
   cd modern-internet-speed-meter
   ```

2. Download and unpack **w64devkit** (64-bit MinGW toolchain):
   - Download the latest zip from [w64devkit releases](https://github.com/skeeto/w64devkit/releases).
   - Extract it into your project folder as `./w64devkit`.

3. Install frontend dependencies:
   ```bash
   npm install
   ```

4. Start the development server:
   ```powershell
   $env:PATH = "$(pwd)\w64devkit\w64devkit\bin;C:\Users\ihars\.cargo\bin;" + $env:PATH
   npm run tauri dev
   ```

5. Build the production release:
   ```powershell
   $env:PATH = "$(pwd)\w64devkit\w64devkit\bin;C:\Users\ihars\.cargo\bin;" + $env:PATH
   npm run tauri build
   ```

---

## License & Attribution

This project is licensed under the **Source-Available Public License (NC-SA + Attribution)** - see the [LICENSE](LICENSE) file for full details:

- **Open Source / Share-Alike:** Any modified versions, forks, or derivative works must remain open source under the exact same terms.
- **Non-Commercial:** Free for personal use. Commercial sale, monetization, or redistribution is strictly prohibited.
- **Preservation of Support and Donation Links:** All derivative works, modifications, and redistributions **MUST** preserve intact and unaltered all original author attributions, branding, and in-app support/donation links (including Ko-fi and Reddit contact links).
- **No Liability:** Provided "AS IS" without warranty of any kind.
