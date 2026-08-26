// Mirrors the two pure operations in the workflow, exercised against the real
// committed ledger. If these are wrong the file gets silently corrupted or a
// contributor is asked to sign twice.
import fs from 'node:fs';
const LEDGER = fs.readFileSync(process.argv[2], 'utf8');

const isRecorded = (ledger, author) =>
  new RegExp(`^\\|\\s*\\[?@?${author}\\b`, 'mi').test(ledger);

function insert(ledger, author, version, when, pr) {
  const row = `| [@${author}](https://github.com/${author}) | ${version} | ${when} | #${pr} |`;
  let next = ledger.replace(/^\| _none yet_ \|.*\n/m, '');
  const lines = next.split('\n');
  let last = -1;
  for (let i = 0; i < lines.length; i++) if (/^\|/.test(lines[i])) last = i;
  if (last === -1) throw new Error('no table');
  lines.splice(last + 1, 0, row);
  return lines.join('\n');
}

let fail = 0;
const ok = (name, cond, detail='') => { console.log(`  ${cond?'ok  ':'FAIL'} ${name}${cond?'':'  '+detail}`); if(!cond) fail++; };

// --- isRecorded ---
ok('fresh ledger has nobody recorded', !isRecorded(LEDGER, 'Aflaungos'));
ok('placeholder row is not read as a person', !isRecorded(LEDGER, 'none'));
// Marius is named in the PROSE — must not be mistaken for a recorded row.
ok('a name in prose is NOT a recorded acceptance', !isRecorded(LEDGER, 'Marius'));

const after = insert(LEDGER, 'Aflaungos', '1.0', '2026-08-27', 110);
ok('after insert, author IS recorded', isRecorded(after, 'Aflaungos'));
ok('placeholder removed', !/_none yet_/.test(after));
ok('row is inside the table', /\|\s*\[@Aflaungos\]/.test(after));
ok('exactly one row for them', (after.match(/\[@Aflaungos\]/g)||[]).length === 1);

// second person appends, does not replace
const after2 = insert(after, 'someone-else', '1.0', '2026-09-01', 111);
ok('first person survives a second insert', isRecorded(after2, 'Aflaungos'));
ok('second person recorded', isRecorded(after2, 'someone-else'));
ok('prose after the table is preserved',
   after2.includes('Marius DAVID') && after2.includes('Before merging a first-time contributor'));

// case-insensitivity: GitHub logins are case-insensitive in practice
ok('match is case-insensitive', isRecorded(after, 'aflaungos'));

console.log(after2.split('\n').filter(l=>l.startsWith('|')).map(l=>'    '+l).join('\n'));
process.exit(fail ? 1 : 0);
