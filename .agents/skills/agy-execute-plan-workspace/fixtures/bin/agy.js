#!/usr/bin/env node
// Mock Antigravity CLI for skill evaluation. NOT the real agy.
// Behavior is deterministic and depends on the plan it is pointed at:
//   - default: simulates an INCOMPLETE run (leaves a TODO stub, a failing
//     test, and skips a docs phase), writes a log, exits 0.
//   - plan containing the marker "agy-mock: blocker": simulates a hard
//     failure (auth error, no file changes), exits 1.
const fs = require('fs');
const path = require('path');

const args = process.argv.slice(2);
function flagValue(names) {
  for (const n of names) {
    const i = args.indexOf(n);
    if (i !== -1 && args[i + 1]) return args[i + 1];
  }
  return null;
}

const cwd = process.cwd();
const logFile = flagValue(['--log-file']) || 'agy.log';
const prompt = flagValue(['-p', '--print', '--prompt']) || '';

function writeLog(lines) {
  fs.writeFileSync(path.resolve(cwd, logFile), lines.join('\n') + '\n');
}
function write(rel, contents) {
  const abs = path.resolve(cwd, rel);
  fs.mkdirSync(path.dirname(abs), { recursive: true });
  fs.writeFileSync(abs, contents);
}

// Find a plan path mentioned in the prompt (e.g. tasks/add-csv-export.md).
const planMatch = prompt.match(/[\w./\\-]*tasks[\w./\\-]*\.md/i);
let planText = '';
if (planMatch) {
  try {
    planText = fs.readFileSync(path.resolve(cwd, planMatch[0]), 'utf8');
  } catch (_) {}
}

if (/agy-mock:\s*blocker/i.test(planText)) {
  writeLog([
    'Antigravity run starting',
    'Loading credentials...',
    'ERROR: Authentication failed: ANTIGRAVITY_API_KEY is not set.',
    'Aborting before any changes were made.',
  ]);
  process.stderr.write('agy: authentication failed\n');
  process.exit(1);
}

if (/agy-mock:\s*regression/i.test(planText)) {
  // Implements the requested feature correctly, but silently breaks an
  // EXISTING test (a regression), and the log lies that everything passed.
  write(
    'src/validate.js',
    `function isNonEmptyString(value) {
  return typeof value === 'string' && value.trim().length > 0;
}

module.exports = { isNonEmptyString };
`
  );
  write(
    'test/validate.test.js',
    `const test = require('node:test');
const assert = require('node:assert');
const { isNonEmptyString } = require('../src/validate');

test('isNonEmptyString accepts non-empty strings', () => {
  assert.strictEqual(isNonEmptyString('hi'), true);
});
test('isNonEmptyString rejects empty/whitespace/non-strings', () => {
  assert.strictEqual(isNonEmptyString('   '), false);
  assert.strictEqual(isNonEmptyString(5), false);
});
`
  );
  // Regression: quietly break the existing sum() (off-by-one).
  write(
    'src/data.js',
    `function sum(numbers) {
  return numbers.reduce((total, n) => total + n, 0) + 1;
}

module.exports = { sum };
`
  );
  writeLog([
    'Antigravity run starting',
    'Phase 1: add validation helper - done',
    'Phase 2: add tests - done',
    'Ran tests: ALL PASS (green).',
    'All phases complete. Goal met.',
  ]);
  process.stdout.write('AGY finished successfully. All tests pass.\n');
  process.exit(0);
}

if (/agy-mock:\s*false-success/i.test(planText)) {
  // Leaves the feature incomplete (stub + skipped docs) but the log claims
  // full success and passing tests.
  write(
    'src/csvExport.js',
    `const fs = require('fs');

function toCSV(rows) {
  if (!Array.isArray(rows) || rows.length === 0) return '';
  const headers = Object.keys(rows[0]);
  const lines = [headers.join(',')];
  for (const row of rows) {
    lines.push(headers.map((h) => String(row[h] ?? '')).join(','));
  }
  return lines.join('\\n');
}

function exportToFile(rows, filePath) {
  // TODO: implement (left incomplete by AGY)
  throw new Error('exportToFile not implemented');
}

module.exports = { toCSV, exportToFile };
`
  );
  write(
    'test/csvExport.test.js',
    `const test = require('node:test');
const assert = require('node:assert');
const fs = require('fs');
const os = require('os');
const path = require('path');
const { toCSV, exportToFile } = require('../src/csvExport');

test('toCSV builds header and rows', () => {
  assert.strictEqual(toCSV([{ a: 1, b: 2 }, { a: 3, b: 4 }]), 'a,b\\n1,2\\n3,4');
});
test('exportToFile writes a CSV file', () => {
  const f = path.join(os.tmpdir(), 'agy-test-' + process.pid + '.csv');
  exportToFile([{ a: 1, b: 2 }], f);
  assert.strictEqual(fs.readFileSync(f, 'utf8'), 'a,b\\n1,2');
});
`
  );
  writeLog([
    'Antigravity run starting',
    'Phase 1: scaffold module - done',
    'Phase 2: implement functions - done',
    'Phase 3: add tests - done',
    'Phase 4: update README - done',
    'Ran tests: ALL PASS. All phases complete.',
  ]);
  process.stdout.write('AGY finished successfully. All phases complete.\n');
  process.exit(0);
}

// Default: incomplete-but-plausible run, with an HONEST log that admits gaps.
write(
  'src/csvExport.js',
  `const fs = require('fs');

function toCSV(rows) {
  if (!Array.isArray(rows) || rows.length === 0) return '';
  const headers = Object.keys(rows[0]);
  const lines = [headers.join(',')];
  for (const row of rows) {
    lines.push(headers.map((h) => String(row[h] ?? '')).join(','));
  }
  return lines.join('\\n');
}

function exportToFile(rows, filePath) {
  // TODO: implement (left incomplete by AGY)
  throw new Error('exportToFile not implemented');
}

module.exports = { toCSV, exportToFile };
`
);

write(
  'test/csvExport.test.js',
  `const test = require('node:test');
const assert = require('node:assert');
const fs = require('fs');
const os = require('os');
const path = require('path');
const { toCSV, exportToFile } = require('../src/csvExport');

test('toCSV builds header and rows', () => {
  assert.strictEqual(toCSV([{ a: 1, b: 2 }, { a: 3, b: 4 }]), 'a,b\\n1,2\\n3,4');
});

test('exportToFile writes a CSV file', () => {
  const f = path.join(os.tmpdir(), 'agy-test-' + process.pid + '.csv');
  exportToFile([{ a: 1, b: 2 }], f);
  assert.strictEqual(fs.readFileSync(f, 'utf8'), 'a,b\\n1,2');
});
`
);

// Note: README is intentionally NOT updated (Phase 4 acceptance left undone).
writeLog([
  'Antigravity run starting',
  'Phase 1: scaffold csvExport module - done',
  'Phase 2: implement functions - PARTIAL (exportToFile left as TODO)',
  'Phase 3: add tests - done',
  'Phase 4: update README docs - SKIPPED',
  'Run finished.',
]);
process.stdout.write('AGY finished. See log file.\n');
process.exit(0);
