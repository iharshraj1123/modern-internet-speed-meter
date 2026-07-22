const fs = require('fs');
const path = require('path');

const buildDir = path.join(__dirname, '..', 'build');

console.log('Cleaning installers and executable artifacts from static build output...');

function cleanDirectory(dirPath) {
  if (!fs.existsSync(dirPath)) return;

  const entries = fs.readdirSync(dirPath, { withFileTypes: true });

  for (const entry of entries) {
    const fullPath = path.join(dirPath, entry.name);

    if (entry.isDirectory()) {
      if (entry.name.toLowerCase() === 'downloads') {
        fs.rmSync(fullPath, { recursive: true, force: true });
        console.log(`Removed directory: ${entry.name}`);
      } else {
        cleanDirectory(fullPath);
      }
    } else if (entry.isFile()) {
      const ext = path.extname(entry.name).toLowerCase();
      if (ext === '.msi' || ext === '.exe' || ext === '.zip' || ext === '.nsis') {
        fs.unlinkSync(fullPath);
        console.log(`Removed installer artifact: ${entry.name}`);
      }
    }
  }
}

try {
  cleanDirectory(buildDir);
  console.log('Build output cleanup completed successfully.');
} catch (err) {
  console.error('Error cleaning installers:', err);
}

