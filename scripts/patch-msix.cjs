const fs = require('fs');
const path = require('path');
const { execSync } = require('child_process');

function getDesktopPath() {
  try {
    const regOut = execSync('powershell -NoProfile -Command "(Get-ItemProperty \'HKCU:\\Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\Shell Folders\').Desktop"', { encoding: 'utf8' }).trim();
    if (regOut && fs.existsSync(regOut)) return regOut;
  } catch (e) {
    // fallback
  }

  const oneDrive = path.join(process.env.USERPROFILE || '', 'OneDrive', 'Desktop');
  if (fs.existsSync(oneDrive)) return oneDrive;
  const standard = path.join(process.env.USERPROFILE || '', 'Desktop');
  if (fs.existsSync(standard)) return standard;
  return process.cwd();
}

function findLatestMsix(desktopDir) {
  const files = fs.readdirSync(desktopDir)
    .filter(f => f.toLowerCase().endsWith('.msix') && f.toLowerCase().includes('harsh.internetspeedmetermodern'))
    .map(f => ({
      name: f,
      path: path.join(desktopDir, f),
      time: fs.statSync(path.join(desktopDir, f)).mtimeMs
    }))
    .sort((a, b) => b.time - a.time);

  return files.length > 0 ? files[0] : null;
}

function patchManifestXml(xmlContent) {
  let updated = xmlContent;

  // 1. Ensure uap5 namespace is defined
  if (!updated.includes('xmlns:uap5=')) {
    updated = updated.replace(
      '<Package ',
      '<Package xmlns:uap5="http://schemas.microsoft.com/appx/manifest/uap/windows10/5" '
    );
  }

  // 2. Ensure uap5 is in IgnorableNamespaces
  const ignorableMatch = updated.match(/IgnorableNamespaces="([^"]+)"/);
  if (ignorableMatch) {
    const namespaces = ignorableMatch[1].split(' ');
    if (!namespaces.includes('uap5')) {
      const newNamespaces = `${ignorableMatch[1]} uap5`;
      updated = updated.replace(ignorableMatch[0], `IgnorableNamespaces="${newNamespaces}"`);
    }
  }

  // 3. Ensure StartupTask extension is present
  if (!updated.includes('windows.startupTask')) {
    const startupExtension = [
      '        <uap5:Extension Category="windows.startupTask">',
      '          <uap5:StartupTask TaskId="InternetSpeedMeterStartup" Enabled="true" DisplayName="Internet Speed Meter" />',
      '        </uap5:Extension>'
    ].join('\r\n');

    if (updated.includes('</Extensions>')) {
      updated = updated.replace('</Extensions>', `${startupExtension}\r\n      </Extensions>`);
    } else if (updated.includes('</Application>')) {
      const extBlock = `      <Extensions>\r\n${startupExtension}\r\n      </Extensions>\r\n    </Application>`;
      updated = updated.replace('</Application>', extBlock);
    }
  }

  return updated;
}

function main() {
  console.log('====================================================');
  console.log('MSIX Manifest Auto-Patcher for Internet Speed Meter');
  console.log('====================================================');

  const desktopDir = getDesktopPath();
  console.log(`Searching for MSIX package in: ${desktopDir}`);

  const targetMsix = findLatestMsix(desktopDir);
  if (!targetMsix) {
    console.error('Error: Could not find any harsh.Internetspeedmetermodern*.msix on your Desktop.');
    console.error('Please make sure you have generated the package using MSIX Packaging Tool first.');
    process.exit(1);
  }

  console.log(`Found package: ${targetMsix.name}`);
  const sevenZipPath = 'C:\\Program Files\\7-Zip\\7z.exe';
  if (!fs.existsSync(sevenZipPath)) {
    console.error(`Error: 7-Zip was not found at ${sevenZipPath}`);
    process.exit(1);
  }

  const workDir = path.join(process.env.TEMP || '.', `msix_patch_${Date.now()}`);
  fs.mkdirSync(workDir, { recursive: true });

  try {
    // 1. Extract AppxManifest.xml from MSIX
    console.log('Extracting AppxManifest.xml from package...');
    execSync(`"${sevenZipPath}" e "${targetMsix.path}" "AppxManifest.xml" -o"${workDir}" -y`, { stdio: 'ignore' });

    const manifestFile = path.join(workDir, 'AppxManifest.xml');
    if (!fs.existsSync(manifestFile)) {
      console.error('Error: Failed to extract AppxManifest.xml from the MSIX package.');
      process.exit(1);
    }

    const originalXml = fs.readFileSync(manifestFile, 'utf8');
    if (originalXml.includes('windows.startupTask') && originalXml.includes('xmlns:uap5=')) {
      console.log('Package already contains windows.startupTask and uap5. No changes needed!');
      console.log('Your MSIX is ready to upload to Microsoft Partner Center.');
      return;
    }

    // 2. Patch XML
    console.log('Injecting uap5 and windows.startupTask extension into manifest...');
    const patchedXml = patchManifestXml(originalXml);
    fs.writeFileSync(manifestFile, patchedXml, 'utf8');

    // 3. Update the package
    console.log('Updating AppxManifest.xml inside the MSIX package...');
    execSync(`"${sevenZipPath}" u "${targetMsix.path}" "${manifestFile}"`, { stdio: 'inherit' });

    console.log('');
    console.log('====================================================');
    console.log('SUCCESS! The MSIX package was automatically patched.');
    console.log(`Package: ${targetMsix.path}`);
    console.log('Startup task is now permanently embedded.');
    console.log('You can now upload it directly to Microsoft Partner Center!');
    console.log('====================================================');
  } catch (err) {
    console.error('Failed to patch MSIX package:', err.message);
    process.exit(1);
  } finally {
    try {
      fs.rmSync(workDir, { recursive: true, force: true });
    } catch (_) {}
  }
}

main();
