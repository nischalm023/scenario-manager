#!/usr/bin/env python3
"""Build Foundry's Map Creator adapter and test its actual WASM component.

Dependency checkout paths are explicit: the guest and host manifests are rendered
in a temporary workspace. Source fixtures and application schemas stay in Foundry.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile


def run(arguments, environment):
    subprocess.run(arguments, check=True, env=environment)


def resolve(manifest, environment):
    subprocess.run(['cargo', 'metadata', '--format-version', '1',
                    '--manifest-path', str(manifest)], check=True, env=environment,
                   stdout=subprocess.DEVNULL)


def main():
    source = Path(__file__).resolve().parent
    foundry = source.parents[2]
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--world-proto', type=Path, required=True)
    parser.add_argument('--target-dir', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True,
                        help='new directory for the tested module and receipt')
    parser.add_argument('--production-dumps', type=Path,
                        help='also run the ignored tests against site-a through site-e')
    args = parser.parse_args()
    runtime, target, output = (p.resolve() for p in (args.world_proto, args.target_dir, args.output))
    for required in ('adapter/sdk/Cargo.toml', 'adapter/host/Cargo.toml', 'lens/rust/Cargo.toml',
                     'proto/core/Cargo.toml', 'server/server/server-defs.json'):
        if not (runtime / required).is_file():
            parser.error(f'--world-proto lacks {required}: {runtime}')
    output.mkdir(parents=True, exist_ok=False)
    environment = dict(os.environ, FOUNDRY_APPLICATION_ROOT=str(foundry), FOUNDRY_WPS_ROOT=str(runtime))
    with tempfile.TemporaryDirectory(prefix='foundry-mapcreator-') as temporary:
        guest = Path(temporary) / 'mapcreator'
        shutil.copytree(source, guest, ignore=shutil.ignore_patterns('target', '__pycache__'))
        for directory in (guest, guest / 'tests'):
            template = directory / 'Cargo.toml.in'
            manifest = template.read_text().replace('@WPS@', str(runtime)).replace('@FOUNDRY@', str(foundry))
            (directory / 'Cargo.toml').write_text(manifest)
        # Keep the reviewed dependency versions while adding the relocated reader.
        shutil.copyfile(runtime / 'Cargo.lock', guest / 'tests/Cargo.lock')
        resolve(guest / 'Cargo.toml', environment)
        run(['cargo', 'test', '--locked', '--manifest-path', str(guest / 'Cargo.toml'),
             '--target-dir', str(target / 'unit')], environment)
        run(['cargo', 'build', '--locked', '--release', '--target', 'wasm32-wasip2',
             '--manifest-path', str(guest / 'Cargo.toml'), '--target-dir', str(target / 'guest')], environment)
        module = output / 'mapcreator.wasm'
        shutil.copyfile(target / 'guest/wasm32-wasip2/release/mapcreator_adapter.wasm', module)
        environment['FOUNDRY_MAPCREATOR_MODULE'] = str(module)
        resolve(guest / 'tests/Cargo.toml', environment)
        command = ['cargo', 'test', '--locked', '--manifest-path', str(guest / 'tests/Cargo.toml'),
                   '--target-dir', str(target / 'host'), '--', '--nocapture']
        run(command, environment)
        if args.production_dumps:
            environment['MAPCREATOR_DUMPS'] = str(args.production_dumps.resolve())
            run(command + ['--ignored'], environment)
        receipt = {
            'foundry': str(foundry), 'world_proto': str(runtime),
            'module_sha256': hashlib.sha256(module.read_bytes()).hexdigest(),
            'production_exports_tested': args.production_dumps is not None,
        }
        # Build evidence belongs beside the output artifact, outside source data.
        (output / 'receipt.txt').write_text(json.dumps(receipt, indent=2) + '\n')


if __name__ == '__main__':
    main()
