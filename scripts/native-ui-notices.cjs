// Include actual licence texts for every production npm dependency, transitively.
// Fail closed if a dependency has no readable licence; never emit runner paths.
const checker = require('../app/node_modules/license-checker');
const fs = require('node:fs');
const path = require('node:path');
checker.init({ start: path.resolve(__dirname, '../app'), production: true, excludePrivatePackages: true }, (error, packages) => {
  if (error) throw error;
  console.log('\nUI npm dependencies\n');
  for (const [name, info] of Object.entries(packages).sort(([a], [b]) => a.localeCompare(b))) {
    if (!info.licenseFile || !fs.statSync(info.licenseFile).isFile()) throw new Error(`Missing licence text: ${name}`);
    console.log(`\n${name}\nLicence: ${info.licenses}\n${info.repository || ''}\n`);
    console.log(fs.readFileSync(info.licenseFile, 'utf8'));
  }
});
