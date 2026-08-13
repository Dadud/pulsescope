import { readFileSync } from 'node:fs';

const matrix = JSON.parse(readFileSync(new URL('../release/acceptance-matrix.json', import.meta.url), 'utf8'));
const allowed = new Set(matrix.status_values ?? []);
const ids = new Set();
const failures = [];

if (matrix.schema_version !== 1) failures.push('acceptance matrix schema_version must be 1');
if (!Array.isArray(matrix.components) || matrix.components.length === 0) failures.push('acceptance matrix has no components');

for (const component of matrix.components ?? []) {
  if (!component.id || ids.has(component.id)) failures.push(`invalid or duplicate component id: ${component.id}`);
  ids.add(component.id);
  if (!allowed.has(component.status)) failures.push(`${component.id}: unsupported status ${component.status}`);
  if (!Array.isArray(component.evidence)) failures.push(`${component.id}: evidence must be an array`);
  if ((component.status === 'hardware_verified' || component.status === 'production') && component.evidence.length === 0) {
    failures.push(`${component.id}: ${component.status} requires evidence`);
  }
}

if (matrix.rules?.decoder_availability_requires !== 'recorded_iq_e2e') {
  failures.push('decoder availability gate must remain recorded_iq_e2e');
}

if (failures.length) {
  console.error(failures.join('\n'));
  process.exit(1);
}

const counts = Object.fromEntries([...allowed].map((status) => [status, matrix.components.filter((item) => item.status === status).length]));
console.log(`verified ${matrix.components.length} release components: ${JSON.stringify(counts)}`);
