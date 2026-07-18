#!/usr/bin/env python3
"""Fail CI when release claims are not backed by structured acceptance evidence."""
import argparse, json, pathlib, re, sys

ROOT = pathlib.Path(__file__).resolve().parents[1]
REQUIRED_DOCS = ['README.md', 'docs/API.md', 'docs/BUILD.md', 'docs/PARSERS.md',
                 'docs/PRESETS.md', 'docs/FEATURE_STATUS.md']
REQUIRED_CASES = {'mock_mode','iq_playback','authenticated_headless','tls','docker',
                  'desktop_startup','verified_sdr','native_decoder_fixtures','recording',
                  'streaming','database_migration','clean_install','upgrade_existing_data',
                  'rollback','uninstall','unicode_and_space_paths'}

def main():
    ap = argparse.ArgumentParser(); ap.add_argument('--release', action='store_true'); args = ap.parse_args()
    matrix = json.loads((ROOT/'release/acceptance-matrix.json').read_text())
    ids = {x['id'] for x in matrix['checks']}
    errors = [f'missing acceptance case: {x}' for x in sorted(REQUIRED_CASES - ids)]
    for doc in REQUIRED_DOCS:
        if not (ROOT/doc).is_file(): errors.append(f'missing required document: {doc}')
    for case in matrix['checks']:
        if case['status'] == 'complete' and not case.get('evidence'):
            errors.append(f"complete case lacks evidence: {case['id']}")
    api_source = (ROOT/'src-tauri/src/api.rs').read_text()
    api_docs = (ROOT/'docs/API.md').read_text()
    routes = set(re.findall(r'\.route\(\s*"([^"]+)"', api_source))
    documented = set(re.findall(r'`(/[^` ]*)`', api_docs))
    for route in sorted(routes - documented):
        errors.append(f'undocumented route: {route}')
    if args.release:
        bad = [x['id'] for x in matrix['checks'] if x['required'] and x['status'] != 'complete']
        if bad: errors.append('required acceptance checks incomplete: ' + ', '.join(bad))
    # Developer home paths are never valid published documentation.
    for doc in REQUIRED_DOCS:
        text = (ROOT/doc).read_text()
        if re.search(r'(?:[A-Za-z]:\\Users\\|/home/[^/< ]+|/Users/[^/< ]+)', text):
            errors.append(f'developer-specific path in {doc}')
    if errors: print('\n'.join('ERROR: '+e for e in errors), file=sys.stderr); return 1
    print(f"release contract valid ({len(matrix['checks'])} acceptance rows)")
    return 0
if __name__ == '__main__': raise SystemExit(main())

