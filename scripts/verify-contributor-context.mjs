import { existsSync, readFileSync } from 'node:fs';

const root = new URL('../', import.meta.url);
const required = [
  'AGENTS.md',
  'CLAUDE.md',
  'GEMINI.md',
  '.github/copilot-instructions.md',
  '.cursor/rules/pulsescope.mdc',
  '.windsurfrules',
  'docs/ARCHITECTURE.md',
  'docs/CONTRIBUTING.md',
  'docs/DECISIONS/README.md',
];
const failures = [];
const read = (path) => readFileSync(new URL(path, root), 'utf8');

for (const path of required) {
  if (!existsSync(new URL(path, root))) failures.push(`missing contributor context: ${path}`);
}

for (const path of ['CLAUDE.md', 'GEMINI.md', '.github/copilot-instructions.md', '.cursor/rules/pulsescope.mdc', '.windsurfrules']) {
  if (existsSync(new URL(path, root)) && !read(path).includes('AGENTS.md')) failures.push(`${path} must point to AGENTS.md`);
}

if (existsSync(new URL('AGENTS.md', root))) {
  const agents = read('AGENTS.md');
  for (const term of ['release/acceptance-matrix.json', 'docs/ARCHITECTURE.md', 'docs/CONTRIBUTING.md', 'Definition of done', 'Handoff template']) {
    if (!agents.includes(term)) failures.push(`AGENTS.md is missing required reference: ${term}`);
  }
}

const readme = read('README.md');
if (/\b\d+ unit tests passing\b/i.test(readme)) failures.push('README must not contain a hand-maintained passing-test count');
if (!readme.includes('docs/FEATURE_STATUS.md')) failures.push('README must link to generated feature status');

if (failures.length) {
  console.error(failures.join('\n'));
  process.exit(1);
}
console.log(`verified ${required.length} contributor context files`);
