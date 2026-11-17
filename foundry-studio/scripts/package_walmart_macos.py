#!/usr/bin/env python3
"""Build the temporary, self-contained Walmart demonstration app for macOS."""

import argparse
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import plistlib
import re
import shutil
import subprocess
import sys
import tempfile


ROOT = Path(__file__).resolve().parents[1]
SPEC = ROOT / "sim/dessim/tests/fixtures/walmart_dfw6_spec.json"
TARGETS = {"arm64": "aarch64-apple-darwin", "x86_64": "x86_64-apple-darwin"}
MINIMUM_MACOS = "13.0"


def run(args, *, env=None, capture=False):
    print("+ " + " ".join(str(arg) for arg in args), flush=True)
    result = subprocess.run(
        [str(arg) for arg in args], cwd=ROOT, env=env, check=True,
        stdout=subprocess.PIPE if capture else None, text=True,
    )
    return result.stdout.strip() if capture else None


def digest(path):
    checksum = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            checksum.update(chunk)
    return checksum.hexdigest()


def write_json(path, value):
    path.write_text(json.dumps(value, indent=2) + "\n", encoding="utf-8")


def copy_scenario(resources):
    """Copy only this spec's runtime dependencies and make every path portable."""
    spec = json.loads(SPEC.read_text(encoding="utf-8"))
    inputs = []

    def copy_input(source_name, relative_name):
        source = (SPEC.parent / source_name).resolve(strict=True)
        destination = resources / relative_name
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(source, destination)
        inputs.append({"bundle_path": relative_name, "sha256": digest(destination)})
        return relative_name

    spec["world"] = copy_input(spec["world"], "walmart_dfw6.json")
    scenario = spec["scenario"]
    for field, destination in (
        ("onhand_csv", "walmart-data/onhand.csv"),
        ("outbound_csv", "walmart-data/outbound.csv"),
        ("stock_top_up_csv", "walmart_demo_stock_top_up.csv"),
    ):
        if field in scenario and scenario[field] is not None:
            scenario[field] = copy_input(scenario[field], destination)
        elif field != "stock_top_up_csv":
            raise ValueError(f"Main Walmart spec is missing {field}")

    if spec.get("settings", {}).get("max_orders") is not None:
        raise ValueError("The distributed main scenario must not have an order-count cap")
    write_json(resources / SPEC.name, spec)
    inputs.append({"bundle_path": SPEC.name, "sha256": digest(resources / SPEC.name)})
    return inputs


def inspect_binary(binary, architectures):
    actual = set(run(["lipo", "-archs", binary], capture=True).split())
    if actual != set(architectures):
        raise RuntimeError(f"Expected architectures {architectures}, got {sorted(actual)}")
    dependencies = set()
    minimum_versions = {}
    for arch in architectures:
        linked = run(["otool", "-arch", arch, "-L", binary], capture=True)
        for line in linked.splitlines():
            if not line.startswith("\t"):
                continue
            dependency = line.strip().split(" (", 1)[0]
            if not dependency.startswith(("/System/Library/", "/usr/lib/")):
                raise RuntimeError(f"Unbundled dependency for {arch}: {dependency}")
            dependencies.add(dependency)
        commands = run(["otool", "-arch", arch, "-l", binary], capture=True)
        version = re.search(r"\bminos\s+(\d+(?:\.\d+)*)", commands)
        if not version:
            raise RuntimeError(f"No macOS deployment minimum found for {arch}")
        minimum_versions[arch] = version[1]
        if tuple(map(int, version[1].split(".")[:2])) > (13, 0):
            raise RuntimeError(f"{arch} requires macOS {version[1]}, above advertised {MINIMUM_MACOS}")
    return sorted(dependencies), minimum_versions


def recipient_readme(version, architectures, signed):
    hardware = "Apple silicon and Intel Macs" if len(architectures) == 2 else "Apple silicon Macs"
    return f"""Walmart Simulation — iteration {version}

Requires macOS {MINIMUM_MACOS} or newer, on {hardware}.

1. Unzip the download and move Walmart Simulation {version}.app to Applications
   or another convenient folder. Keep the .app intact.
2. Double-click the app. It loads the complete Walmart map and order workload.
3. Click Run to start. Opening the app does not start the simulation.

No command-line arguments, repository checkout, Rust, Python, or extra data
downloads are required. This is a temporary Walmart demonstration build.
Each new iteration is a separate app so older versions can be kept for comparison.
The version is shown in the app's title. Share that version when reporting issues.

The starting inventory is automatically placed from the on-hand extract while
Vinay's mock-put file is pending. The bundled main spec states any additional
stock overlay explicitly; the original on-hand and outbound CSVs are unchanged.

Signing: {signed}. This package has NOT been notarized by Apple.
If macOS blocks the first launch, use System Settings > Privacy & Security >
Open Anyway for this app, provided you trust the source of this copy. A managed
Mac may require assistance from your IT administrator. Apple describes this at:
https://support.apple.com/en-us/102445

The app runs locally. The full all-order simulation has not yet been validated
to completion; retain the iteration number and any error shown during a demo.
"""


def package(args):
    if sys.platform != "darwin":
        raise RuntimeError("Build this macOS app on a Mac with Xcode command-line tools")
    if args.iteration < 1:
        raise ValueError("--iteration must be a positive integer")
    version = f"{args.iteration:03d}"
    release_name = f"Walmart-Simulation-{version}"
    output = args.output_dir.expanduser().resolve()
    destination = output / release_name
    if destination.exists():
        raise FileExistsError(f"Refusing to overwrite {destination}; choose a new iteration")
    architectures = list(TARGETS) if args.arch == "universal" else ["arm64"]
    commit = run(["git", "rev-parse", "HEAD"], capture=True)
    branch = run(["git", "rev-parse", "--abbrev-ref", "HEAD"], capture=True)
    dirty = bool(run(["git", "status", "--porcelain", "--untracked-files=no"], capture=True))
    built_at = datetime.now(timezone.utc).isoformat(timespec="seconds").replace("+00:00", "Z")
    env = os.environ.copy()
    target_dir = Path(env.get("CARGO_TARGET_DIR", ROOT / "target")).expanduser().resolve()
    env.update({
        "MACOSX_DEPLOYMENT_TARGET": MINIMUM_MACOS,
        "CARGO_TARGET_DIR": str(target_dir),
        "WALMART_DEMO_VERSION": version,
        "WALMART_DEMO_COMMIT": commit,
        "WALMART_DEMO_BRANCH": branch,
        "WALMART_DEMO_DIRTY": str(dirty).lower(),
        "WALMART_DEMO_BUILT_AT": built_at,
    })
    binaries = []
    for arch in architectures:
        target = TARGETS[arch]
        run(["cargo", "build", "--release", "--locked", "-p", "desvis", "--features", "walmart-demo",
             "--target", target], env=env)
        binaries.append(target_dir / target / "release/desvis")

    output.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix=f".{release_name}-", dir=output) as scratch:
        staging = Path(scratch) / release_name
        staging.mkdir()
        app = staging / f"Walmart Simulation {version}.app"
        contents = app / "Contents"
        executable = contents / "MacOS/desvis"
        executable.parent.mkdir(parents=True)
        resources = contents / "Resources/walmart"
        resources.mkdir(parents=True)
        if len(binaries) == 1:
            shutil.copyfile(binaries[0], executable)
        else:
            run(["lipo", "-create", *binaries, "-output", executable])
        executable.chmod(0o755)
        dependencies, minimum_versions = inspect_binary(executable, architectures)
        inputs = copy_scenario(resources)
        with (contents / "Info.plist").open("wb") as stream:
            plistlib.dump({
                "CFBundleDevelopmentRegion": "en",
                "CFBundleDisplayName": f"Walmart Simulation {version}",
                "CFBundleName": f"Walmart Simulation {version}",
                "CFBundleExecutable": "desvis",
                "CFBundleIdentifier": f"com.greyorange.walmart-simulation.iteration{version}",
                "CFBundleInfoDictionaryVersion": "6.0",
                "CFBundlePackageType": "APPL",
                "CFBundleShortVersionString": f"0.1.{args.iteration}",
                "CFBundleVersion": str(args.iteration),
                "LSMinimumSystemVersion": MINIMUM_MACOS,
                "NSHighResolutionCapable": True,
            }, stream)
        signed = args.sign_identity or "ad-hoc (no Developer ID)"
        readme = recipient_readme(version, architectures, signed)
        (staging / "README.txt").write_text(readme, encoding="utf-8")
        (contents / "Resources/README.txt").write_text(readme, encoding="utf-8")
        manifest = {
            "iteration": version, "commit": commit, "branch": branch, "dirty": dirty,
            "built_at": built_at, "architectures": architectures,
            "minimum_macos": MINIMUM_MACOS, "binary_minimum_macos": minimum_versions,
            "source_spec_sha256": digest(SPEC),
            "inputs": inputs, "system_dependencies": dependencies,
            "signing_identity": signed, "notarized": False,
            "manual_run": True, "order_count_cap": None,
        }
        write_json(contents / "Resources/build-manifest.json", manifest)
        signing = ["codesign", "--force", "--sign", args.sign_identity or "-"]
        if args.sign_identity:
            signing += ["--options", "runtime", "--timestamp"]
        run([*signing, app])
        run(["codesign", "--verify", "--deep", "--strict", "--verbose=2", app])
        archive = staging / f"{release_name}.zip"
        run(["ditto", "-c", "-k", "--sequesterRsrc", "--keepParent", app, archive])
        manifest["executable_sha256"] = digest(executable)
        manifest["zip_sha256"] = digest(archive)
        write_json(staging / "manifest.json", manifest)
        checksums = "".join(
            f"{digest(path)}  {path.relative_to(staging).as_posix()}\n"
            for path in sorted(staging.rglob("*")) if path.is_file()
        )
        (staging / "SHA256SUMS.txt").write_text(checksums, encoding="utf-8")
        if destination.exists():
            raise FileExistsError(f"Release appeared during build; refusing to overwrite {destination}")
        staging.rename(destination)
    print(f"\nCreated {destination}\nShare {destination / archive.name}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--iteration", type=int, required=True, help="Sequential distribution number; never overwritten")
    parser.add_argument("--arch", choices=("universal", "arm64"), default="universal")
    parser.add_argument("--output-dir", type=Path, default=ROOT / "dist/walmart-demo")
    parser.add_argument("--sign-identity", help="Optional installed Developer ID signing identity; otherwise ad-hoc")
    args = parser.parse_args()
    try:
        package(args)
    except (OSError, ValueError, RuntimeError, subprocess.CalledProcessError) as error:
        parser.exit(1, f"Packaging failed: {error}\n")


if __name__ == "__main__":
    main()
