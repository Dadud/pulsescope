import { readFileSync } from 'node:fs';

const matrixUrl = new URL('../release/acceptance-matrix.json', import.meta.url);
const matrix = JSON.parse(readFileSync(matrixUrl, 'utf8'));
const allowed = new Set(matrix.status_values ?? []);
const visibility = new Set(matrix.visibility_values ?? []);
const ids = new Set();
const failures = [];
const rank = new Map(['planned', 'development', 'fixture_verified', 'hardware_verified', 'production'].map((v, i) => [v, i]));

if (matrix.schema_version !== 2) failures.push('acceptance matrix schema_version must be 2');
if (!Array.isArray(matrix.components) || matrix.components.length === 0) failures.push('acceptance matrix has no components');
if (matrix.rules?.decoder_availability_requires !== 'recorded_iq_e2e') failures.push('decoder availability gate must remain recorded_iq_e2e');

for (const component of matrix.components ?? []) {
  if (!component.id || !/^[a-z0-9-]+$/.test(component.id) || ids.has(component.id)) failures.push(`invalid or duplicate component id: ${component.id}`);
  ids.add(component.id);
  if (!component.name || !component.group) failures.push(`${component.id}: name and group are required`);
  if (!allowed.has(component.status)) failures.push(`${component.id}: unsupported status ${component.status}`);
  if (!visibility.has(component.visibility)) failures.push(`${component.id}: unsupported visibility ${component.visibility}`);
  if (typeof component.required !== 'boolean') failures.push(`${component.id}: required must be boolean`);
  if (!component.acceptance_gate || component.acceptance_gate.length < 20) failures.push(`${component.id}: a concrete acceptance_gate is required`);
  if (!Array.isArray(component.evidence)) failures.push(`${component.id}: evidence must be an array`);
  for (const evidence of component.evidence ?? []) {
    if (!evidence || typeof evidence !== 'object' || !evidence.type || !evidence.ref) failures.push(`${component.id}: evidence entries require type and ref`);
  }
  if (['fixture_verified', 'hardware_verified', 'production'].includes(component.status) && component.evidence.length === 0) {
    failures.push(`${component.id}: ${component.status} requires structured evidence`);
  }
  if (['hardware_verified', 'production'].includes(component.status)) {
    const hardwareEvidence = component.evidence.some((item) => item.type === 'hardware_run' && item.date && item.device);
    if (component.status === 'hardware_verified' && !hardwareEvidence) failures.push(`${component.id}: hardware verification requires dated device evidence`);
  }
  const minimum = matrix.rules?.normal_ui_minimum_status;
  if (component.visibility === 'normal' && rank.get(component.status) < rank.get(minimum)) {
    failures.push(`${component.id}: normal UI items must be at least ${minimum}`);
  }
}

for (const certifiedId of matrix.rules?.certified_hardware ?? []) {
  const component = matrix.components.find((item) => item.id === certifiedId);
  if (!component || !['hardware_verified', 'production'].includes(component.status)) failures.push(`${certifiedId}: certified hardware must be hardware_verified or production`);
}

const apiSource = readFileSync(new URL('../src-tauri/src/api.rs', import.meta.url), 'utf8');
const decoderUi = readFileSync(new URL('../ui/src/routes/feature-packs/+page.svelte', import.meta.url), 'utf8');
for (const decoderId of matrix.rules?.decoder_catalog_required_ids ?? []) {
  const listed =
    apiSource.includes(`decoder_development_entry("${decoderId}"`) ||
    apiSource.includes(`decoder_fixture_verified_entry("${decoderId}"`);
  if (!listed) failures.push(`decoder catalog is missing ${decoderId}`);
}
if (!decoderUi.includes('missing_gate') || !decoderUi.includes('Beta')) failures.push('normal decoder UI must show beta status and missing gate');

if (failures.length) {
  console.error(failures.join('\n'));
  process.exit(1);
}

const counts = Object.fromEntries([...allowed].map((status) => [status, matrix.components.filter((item) => item.status === status).length]));
console.log(`verified ${matrix.components.length} release components: ${JSON.stringify(counts)}`);
