# Version Update & Release SOP Guide (gemini.md)

This document is the standard operating procedure (SOP) for bumping application versions in **Internet Speed Meter** to ensure that no configuration, manifest, UI element, or documentation is left behind.

---

## 1. Version Locations Checklist

When bumping the application version from `X.Y.Z` to `A.B.C`, update the following **6 files**:

### 1. `src-tauri/tauri.conf.json`
* **Field:** `"version": "A.B.C"` (under root object)
* **Purpose:** Core Tauri application configuration. Used by the bundler (NSIS/MSI) and exposed to frontend via Tauri APIs.

### 2. `src-tauri/Cargo.toml`
* **Field:** `version = "A.B.C"` (under `[package]`)
* **Purpose:** Rust backend crate version.

### 3. `package.json`
* **Field:** `"version": "A.B.C"`
* **Purpose:** Node.js / Vite / SvelteKit frontend package version.

### 4. `AppxManifest.xml`
* **Field:** `Version="A.B.C.0"` (under `<Identity>`)
* **Purpose:** Windows MSIX / Microsoft Store package manifest (requires 4-part quad format: `Major.Minor.Build.Revision`).

### 5. `src/routes/settings/+page.svelte`
* **Field:** `let appVersion = $state("A.B.C");`
* **Purpose:** Settings UI under the **Support Us** tab (`<span class="version-number">Version {appVersion}</span>`).
* *Note:* The page attempts to dynamically read the version via `@tauri-apps/api/app` `getVersion()`, but this default state guarantees immediate and accurate display even before async mount.

### 6. `README.md`
* **Shield Badge:** Update badge URL release tag (`releases/tag/vA.B.C`).
* **Download Links:** Update the release download URL (`releases/tag/vA.B.C`).
* **Installer Filenames:** Update filenames in the download list:
  - `Internet Speed Meter_A.B.C_x64-setup.exe`
  - `Internet Speed Meter_A.B.C_x64_en-US.msi`

---

## 2. Verification Command (Crucial Step)

Before building or committing, run `git grep` to verify that **zero references** to the old version remain:

```powershell
git grep -n "X.Y.Z"
```

If any documentation, UI, or config file still contains the old version number, update it before proceeding.

Also update `src-tauri/Cargo.lock` by running:
```powershell
$env:PATH = "d:\ProjectsNew\appDev\InternetSpeedMeter\w64devkit\w64devkit\bin;$env:USERPROFILE\.cargo\bin;$env:PATH"
cargo check --manifest-path src-tauri/Cargo.toml
```

---

## 3. Production Build Procedure

Run the production build with the local MinGW toolchain in `PATH`:

```bat
build.bat
```
*Or via PowerShell:*
```powershell
$env:PATH = "d:\ProjectsNew\appDev\InternetSpeedMeter\w64devkit\w64devkit\bin;$env:USERPROFILE\.cargo\bin;$env:PATH"
npm run tauri build
```

### Generated Artifacts
Once complete, the installers will be located in:
* **MSI:** `src-tauri\target\release\bundle\msi\Internet Speed Meter_A.B.C_x64_en-US.msi`
* **NSIS Setup:** `src-tauri\target\release\bundle\nsis\Internet Speed Meter_A.B.C_x64-setup.exe`
* **Executable:** `src-tauri\target\release\internet-speed-meter.exe`

---

## 4. Git Commit & Push Workflow

1. Stage all modified configuration and source files:
   ```powershell
   git add package.json src-tauri/Cargo.toml src-tauri/Cargo.lock src-tauri/tauri.conf.json AppxManifest.xml src/routes/settings/+page.svelte README.md
   ```

2. Commit with standard release message format:
   ```powershell
   git commit -m "Release vA.B.C: <summary of key fixes/features>"
   ```

3. Pull rebase to stay in sync with origin:
   ```powershell
   git pull --rebase origin main
   ```

4. Push to remote:
   ```powershell
   git push origin main
   ```

---

## 5. Publishing GitHub Release with Installers

Use the GitHub CLI (`gh`) to create the release tag and attach both installers:

```powershell
gh release create vA.B.C `
  "src-tauri\target\release\bundle\msi\Internet Speed Meter_A.B.C_x64_en-US.msi" `
  "src-tauri\target\release\bundle\nsis\Internet Speed Meter_A.B.C_x64-setup.exe" `
  --title "vA.B.C - Release Title" `
  --notes "Release notes and change summary..."
```

Verify release on GitHub:
```powershell
gh release view vA.B.C
```

---

## 6. Microsoft Store / Partner Center Packaging

If updating the Microsoft Store listing:
1. Ensure `AppxManifest.xml` has version `A.B.C.0`.
2. Package MSIX using `patch_msix.bat` or MSIX Packaging Tool.
3. Keep the debug symbols file (`src-tauri\target\release\internet-speed-meter.pdb`) to upload to Microsoft Partner Center symbol server if requested for crash diagnostics.
