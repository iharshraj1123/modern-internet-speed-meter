const fs = require('fs');
const path = require('path');

const buildDir = path.join(__dirname, '..', 'build');
const downloadsDir = path.join(buildDir, 'downloads');
const msiFile = path.join(buildDir, 'setup.msi');

console.log('Cleaning installers from static build output...');

try {
  if (fs.existsSync(downloadsDir)) {
    fs.rmSync(downloadsDir, { recursive: true, force: true });
    console.log('Removed build/downloads directory successfully.');
  }
  if (fs.existsSync(msiFile)) {
    fs.rmSync(msiFile, { force: true });
    console.log('Removed build/setup.msi successfully.');
  }
} catch (err) {
  console.error('Error cleaning installers:', err);
}
