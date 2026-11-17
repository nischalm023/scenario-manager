#!/usr/bin/env python3
"""Build baseline Foundry generator callers and compare their WASM/native behavior."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile


def run(command, environment):
    subprocess.run(command, check=True, env=environment)


def main():
    source = Path(__file__).resolve().parent
    foundry = source.parents[1]
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--world-proto', type=Path, required=True)
    parser.add_argument('--target-dir', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--known-geometry-regressions', action='store_true',
                        help='run both tracked ignored geometry regressions instead of normal host tests')
    parser.add_argument('--test', action='append', choices=['carrier_pool', 'cohort', 'parity', 'rack', 'stamp', 'tiling'],
                        help='run only this host test group; may be repeated')
    args = parser.parse_args()
    if args.test and args.known_geometry_regressions:
        parser.error('--test and --known-geometry-regressions select different test modes')
    for lens in ('cell_grid', 'stamped', 'racked', 'cohorted', 'pooled'):
        definition = json.loads((foundry / f'data/seed/application/lenses/{lens}.json').read_text())
        text_path = foundry / 'data/seed/application' / definition['text']['path']
        if hashlib.sha256(text_path.read_bytes()).hexdigest() != definition['text']['sha256']:
            raise ValueError(f'{lens}: input lens content does not match its definition hash')
    runtime, target, output = (p.resolve() for p in (args.world_proto, args.target_dir, args.output))
    output.mkdir(parents=True, exist_ok=False)
    environment = dict(os.environ, FOUNDRY_PARITY_ROOT=str(foundry), FOUNDRY_PARITY_MODULES=str(output))
    with tempfile.TemporaryDirectory(prefix='foundry-generators-') as temporary:
        guest = Path(temporary) / 'generators'
        shutil.copytree(source, guest, ignore=shutil.ignore_patterns('target', '__pycache__'))
        for directory in (guest, guest / 'tests'):
            text = (directory / 'Cargo.toml.in').read_text()
            (directory / 'Cargo.toml').write_text(text.replace('@WPS@', str(runtime)).replace('@FOUNDRY@', str(foundry)))
        shutil.copyfile(runtime / 'Cargo.lock', guest / 'tests/Cargo.lock')
        for manifest in (guest / 'Cargo.toml', guest / 'tests/Cargo.toml'):
            subprocess.run(['cargo', 'metadata', '--format-version', '1', '--manifest-path', str(manifest)],
                           check=True, env=environment, stdout=subprocess.DEVNULL)
        run(['cargo', 'build', '--locked', '--release', '--target', 'wasm32-wasip2',
             '--manifest-path', str(guest / 'Cargo.toml'), '--target-dir', str(target / 'guest')], environment)
        run(['cargo', 'test', '--locked', '--package', 'seed-identity',
             '--manifest-path', str(guest / 'Cargo.toml'), '--target-dir', str(target / 'identity')], environment)
        modules = {}
        for name in ('rack', 'stamp', 'tiling', 'cohort', 'carrier_pool'):
            module = target / f'guest/wasm32-wasip2/release/seed_generator_{name}.wasm'
            output_module = output / f'{name}.wasm'
            shutil.copyfile(module, output_module)
            modules[name] = hashlib.sha256(output_module.read_bytes()).hexdigest()
        command = ['cargo', 'test', '--locked', '--manifest-path', str(guest / 'tests/Cargo.toml'),
                   '--target-dir', str(target / 'host')]
        if args.known_geometry_regressions:
            command += ['--test', 'parity', '--', '--ignored', '--nocapture']
        else:
            for group in args.test or []:
                command += ['--test', group]
            command += ['--', '--nocapture']
        run(command, environment)
        (output / 'receipt.txt').write_text(json.dumps({'modules': modules, 'foundry': str(foundry),
                                                     'world_proto': str(runtime)}, indent=2) + '\n')


if __name__ == '__main__':
    main()
