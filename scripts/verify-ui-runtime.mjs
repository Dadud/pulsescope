import { readdir, readFile } from 'node:fs/promises';
import { join } from 'node:path';

const root = join(process.cwd(), 'ui', 'build', '_app', 'immutable');
const required = ['pulsescope:spectrum', 'visibilitychange', 'runtime polling failed'];
let bundle = '';

async function collect(directory) {
  for (const entry of await readdir(directory, { withFileTypes: true })) {
    const path = join(directory, entry.name);
    if (entry.isDirectory()) await collect(path);
    else if (entry.name.endsWith('.js')) bundle += await readFile(path, 'utf8');
  }
}

await collect(root);
const missing = required.filter((marker) => !bundle.includes(marker));
if (missing.length) {
  throw new Error(`production UI omitted receiver startup code: ${missing.join(', ')}`);
}
console.log('verified production receiver startup code');
