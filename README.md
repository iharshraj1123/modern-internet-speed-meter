# Internet Speed Meter & Telemetry Dashboard

A premium, glassmorphic desktop widget and statistics dashboard built with **Tauri v2**, **Rust**, and **SvelteKit** to monitor and log network usage per process.

## 🚀 How to Build & Run locally

### Prerequisites

1. **Node.js**: Install [Node.js](https://nodejs.org/) (LTS recommended).
2. **Rust**: Install Rust via [rustup](https://rustup.rs/).
3. **64-bit C/C++ Compiler (Windows)**:
   This project uses Rust's `x86_64-pc-windows-gnu` target. You need a 64-bit MinGW toolchain installed on your PC.

### Quick Setup (from Clone)

1. Clone the repository:
   ```bash
   git clone <repository-url>
   cd internetspeedmeter
   ```

2. Download and unpack **w64devkit** (64-bit MinGW):
   * Download the latest zip from [w64devkit releases](https://github.com/skeeto/w64devkit/releases) (e.g. `w64devkit-1.20.0.zip`).
   * Extract it inside your project directory as `./w64devkit`.

3. Install frontend dependencies:
   ```bash
   npm install
   ```

4. Prepend the local MinGW compiler to your path and start the Tauri development server:
   * **PowerShell**:
     ```powershell
     $env:PATH = "$(pwd)\w64devkit\w64devkit\bin;C:\Users\ihars\.cargo\bin;" + $env:PATH
     npm run tauri dev
     ```
   * **Command Prompt (CMD)**:
     ```cmd
     set PATH=%CD%\w64devkit\w64devkit\bin;%USERPROFILE%\.cargo\bin;%PATH%
     npm run tauri dev
     ```

## 📦 Building Production Release

To bundle the application into a standalone Windows installer or executable:

1. Prepend the local MinGW compiler to your path (same as Step 4 above).
2. Run the build command:
   ```bash
   npm run tauri build
   ```
   The compiled assets and installer will be located in `src-tauri/target/release/bundle/`.

## 📜 License

This project is licensed under the **Source-Available Public License (NC-SA + Attribution)** - see the [LICENSE](LICENSE) file for full details:
- **Open Source / Share-Alike:** Derivative works must remain open source under the same license terms.
- **Non-Commercial:** Free for personal use. Commercial sale or redistribution is prohibited.
- **Support Links:** Original author attribution and donation/support links must remain intact in all derivative works.
- **No Liability:** Provided "AS IS" without warranty of any kind.

