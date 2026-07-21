# ⚡ Internet Speed Meter & Telemetry Dashboard

[![Latest Release](https://img.shields.io/github/v/release/iharshraj1123/modern-internet-speed-meter?color=10b981&label=Release&style=for-the-badge)](https://github.com/iharshraj1123/modern-internet-speed-meter/releases/tag/v1.1.0)
[![License](https://img.shields.io/badge/License-NC--SA%20%2B%20Attribution-violet?style=for-the-badge)](LICENSE)
[![Platform](https://img.shields.io/badge/Platform-Windows%20x64-sky?style=for-the-badge)](https://github.com/iharshraj1123/modern-internet-speed-meter/releases)

A premium, glassmorphic desktop widget and network telemetry dashboard built with **Tauri v2**, **Rust**, and **SvelteKit**. Features real-time speed monitoring, per-process bandwidth tracking, customizable overlay graph layouts, auto-collapsing widget modes, and historical usage logging.

---

## ✨ Features & Highlights

- **🖥️ Floating Desktop Overlay Widget**:
  - Real-time download & upload transfer rates with smooth SVG graph curves.
  - **Graph Representation Layouts**: Dual Combined Overlay, Stacked Separate Graphs, Download Only, Upload Only, or Hidden Mode.
  - **Vertical Auto-Collapse**: Hidden mode collapses window to 34px height and locks vertical resizing while remembering custom expanded dimensions.
  - Customizable transparency, custom line styles (Dashed vs. Smooth Solid), and optional Peak Speed tag.
  - Click-Through Mode & Position Locking.

- **📊 Comprehensive Telemetry Dashboard**:
  - **Live Section**: 60-second real-time network graphs, active transfer speed averages (`Avg ↓` / `Avg ↑`), peak speed badges (`Peak ↓` / `Peak ↑`), X-axis time scale markers, and process network tables.
  - **Process Analytics**: Per-process bandwidth usage tracking.
  - **Historical Reports**: Daily, monthly, and hourly data usage analytics stored locally in SQLite.

- **🎨 Modern Design & Customization**:
  - 6 Curated Accent Palettes (*Emerald, Violet, Sky, Amber, Rose, Coral*).
  - Explicit Light / Dark / System theme support.
  - Flexible Measurement Units (*Decimal Bytes, Binary Bytes, Decimal Bits, Binary Bits*).
  - Reorganized **Appearance Sub-Tabs** (*Theme & Interface* and *Widget & Graphs*).

- **⚙️ Power & Quotas**:
  - Daily & Monthly Data Budget caps with progress indicators.
  - Global hotkey support for instant widget toggle.
  - System Tray integration with quick context menu.

---

## 📥 Download & Installation

Download the latest installer executable for Windows:

👉 **[Download Version 1.1.0 Release](https://github.com/iharshraj1123/modern-internet-speed-meter/releases/tag/v1.1.0)**

Available formats:
- **`Internet Speed Meter_1.1.0_x64-setup.exe`** (NSIS Windows Installer)
- **`Internet Speed Meter_1.1.0_x64_en-US.msi`** (MSI Installer)

---

## 🚀 Building locally

### Prerequisites

1. **Node.js**: Install [Node.js](https://nodejs.org/) (LTS recommended).
2. **Rust**: Install Rust via [rustup](https://rustup.rs/).
3. **64-bit C/C++ Compiler (Windows)**:
   This project uses Rust's `x86_64-pc-windows-gnu` target.

### Quick Setup

1. **Clone repository**:
   ```bash
   git clone https://github.com/iharshraj1123/modern-internet-speed-meter.git
   cd modern-internet-speed-meter
   ```

2. **Unpack w64devkit** (64-bit MinGW toolchain):
   - Download [w64devkit releases](https://github.com/skeeto/w64devkit/releases).
   - Extract it inside the project directory as `./w64devkit`.

3. **Install NPM dependencies**:
   ```bash
   npm install
   ```

4. **Run Development Server**:
   ```powershell
   $env:PATH = "$(pwd)\w64devkit\w64devkit\bin;C:\Users\ihars\.cargo\bin;" + $env:PATH
   npm run tauri dev
   ```

5. **Build Production Release**:
   ```powershell
   $env:PATH = "$(pwd)\w64devkit\w64devkit\bin;C:\Users\ihars\.cargo\bin;" + $env:PATH
   npm run tauri build
   ```

---

## 📜 License & Attribution

This project is licensed under the **Source-Available Public License (NC-SA + Attribution)** - see the [LICENSE](LICENSE) file for full details:

- **Open Source / Share-Alike:** Derivative works and forks must remain open source under the exact same terms.
- **Non-Commercial:** Free for personal use. Commercial sale, monetization, or redistribution is strictly prohibited.
- **Preservation of Support and Donation Links:** All derivative works, modifications, and redistribution **MUST** preserve intact and unaltered all original author attributions, branding, and in-app support/donation links (including Ko-fi and Reddit contact links).
- **No Liability:** Provided "AS IS" without warranty of any kind.
