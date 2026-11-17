#!/usr/bin/env python3
"""Find qualified-path uses of types in Rust sources (e.g. `crate::sim::engine::Clock`
used inline instead of importing the type with `use`).

Scans git-tracked .rs files, skips `use`/`mod`/`extern crate` items, comments, and
string literals, then reports every remaining path whose final segment is a type-like
identifier (starts with an uppercase letter) preceded by one or more lowercase module
segments.

Usage:
  scripts/find-qualified-paths.py                 # grouped summary (count + path)
  scripts/find-qualified-paths.py --list          # every occurrence as file:line:col
  scripts/find-qualified-paths.py --check [file]  # gate mode: exit 1 on fixable hits
  scripts/find-qualified-paths.py --min-quals 2   # only deep paths (>= 2 `::` before the type)
  scripts/find-qualified-paths.py sim/dessim   # restrict to given paths

Check mode classifies every hit:
  fix     — fails the gate; import the type instead
  keep    — reported, never fails: macro_rules! bodies (paths there resolve at
            each expansion site) and lines carrying a `// keep-qualified: <why>`
            waiver comment
  allowed — suppressed entirely (fmt/io Result families, mpsc, egui/wgpu/eframe,
            derive position, ... — see ALLOWED_* below)
"""

import argparse
import collections
import os
import re
import subprocess
import sys

# Lowercase module segments (incl. crate/self/super/std/...) followed by a
# type-like final segment. The lookbehind rejects matches that are the tail of a
# longer path or identifier (e.g. the `fmt::Result` inside `std::fmt::Result` or
# a `Foo::assoc::Bar` chain).
QUALIFIED_TYPE = re.compile(
    r"(?<![\w:])((?:(?:r#)?[a-z_][a-z0-9_]*::)+[A-Z][A-Za-z0-9_]*)"
)

USE_ITEM = re.compile(r"^\s*(pub(\s*\([^)]*\))?\s+)?use\s")
OTHER_ITEM_SKIP = re.compile(r"^\s*(pub(\s*\([^)]*\))?\s+)?(mod\s|extern\s+crate\s)")

# `u32::MAX`, `f64::consts::PI` etc. are idiomatic — not paths anyone imports away.
PRIMITIVES = {
    "bool", "char", "str",
    "i8", "i16", "i32", "i64", "i128", "isize",
    "u8", "u16", "u32", "u64", "u128", "usize",
    "f32", "f64",
}

SCREAMING_CASE = re.compile(r"^[A-Z][A-Z0-9_]*$")

# Idiomatic qualified uses that stay allowed (suppress with --all):
#  - the fmt/io Result families: `fmt::Result` and `io::Result` exist precisely
#    to avoid shadowing the prelude `Result`, and `impl std::fmt::Display for
#    ...` headers are conventional
#  - `mpsc::Sender` etc.: the std-docs convention (`use std::sync::mpsc;`)
#  - `std::error::Error`: the `Box<dyn std::error::Error>` idiom
#  - egui/wgpu/eframe: upstream docs and the wider ecosystem keep these
#    qualified (`egui::Ui`, `wgpu::BufferUsages`) — their flat namespaces are
#    full of collision-prone names like Rect, Buffer, Color32
ALLOWED_PREFIXES = (
    "fmt::",
    "std::fmt::",
    "io::",
    "std::io::Result",
    "mpsc::",
    "egui::",
    "wgpu::",
    "eframe::",
)
# `syn::Result` is the proc-macro world's `fmt::Result`: it shadows the prelude
# `Result`, so macro crates that use both keep it qualified.
ALLOWED_PATHS = {"std::error::Error", "syn::Result"}

# Attribute/derive position (`#[derive(serde::Serialize)]`, `#[serde(...)]`):
# qualifying there is the convention that keeps macro imports out of scope.
ATTRIBUTE_LINE = re.compile(r"^\s*#!?\[")

MACRO_RULES = re.compile(r"^\s*macro_rules!")

# Per-line waiver: a trailing `// keep-qualified: <reason>` comment exempts the
# line's hits from --check. For the rare type that must stay qualified (e.g.
# two same-named types compared side by side) where no ALLOWED_* rule fits.
KEEP_MARKER = "keep-qualified"


def is_allowed(path):
    return path.startswith(ALLOWED_PREFIXES) or path in ALLOWED_PATHS


def is_type_path(path):
    first = path.split("::", 1)[0]
    final = path.rsplit("::", 1)[1]
    if first in PRIMITIVES:
        return False
    # SCREAMING_SNAKE finals are constants/statics, not types (single capital
    # letters like `T` stay in — those are real type names).
    if len(final) > 1 and SCREAMING_CASE.match(final):
        return False
    return True


def tracked_rust_files(paths):
    # Resolve symlinked paths: git ls-files won't follow them.
    paths = [os.path.relpath(os.path.realpath(p)) for p in paths]
    cmd = ["git", "ls-files", "-z", "--", *(paths or ["."])]
    out = subprocess.run(cmd, capture_output=True, check=True).stdout
    files = [f.decode() for f in out.split(b"\0") if f.endswith(b".rs")]
    # An explicitly named .rs file that exists but isn't tracked yet (a fresh
    # Write before any git add) still gets scanned — the edit-time hook relies
    # on this.
    files.extend(
        p for p in paths if p.endswith(".rs") and p not in files and os.path.isfile(p)
    )
    return files


def strip_noise(line, in_block_comment):
    """Blank out comments and string literals so paths inside them don't match.

    Returns (cleaned_line, in_block_comment). Replacement uses spaces so column
    numbers of surviving matches stay accurate. Raw strings are handled only for
    the common r"..." / r#"..."# single-line forms.
    """
    out = []
    i, n = 0, len(line)
    while i < n:
        if in_block_comment:
            end = line.find("*/", i)
            if end == -1:
                out.append(" " * (n - i))
                return "".join(out), True
            out.append(" " * (end + 2 - i))
            i = end + 2
            in_block_comment = False
            continue
        two = line[i : i + 2]
        if two == "//":
            out.append(" " * (n - i))
            return "".join(out), False
        if two == "/*":
            in_block_comment = True
            i += 2
            out.append("  ")
            continue
        ch = line[i]
        if ch == '"' or (ch == "r" and re.match(r'r#*"', line[i:])):
            if ch == "r":
                hashes = len(line[i:]) - len(line[i:].lstrip("r").lstrip("#")) - 1
                closer = '"' + "#" * hashes
                end = line.find(closer, i + hashes + 2)
                end = n if end == -1 else end + len(closer)
            else:
                j = i + 1
                while j < n:
                    if line[j] == "\\":
                        j += 2
                        continue
                    if line[j] == '"':
                        break
                    j += 1
                end = min(j + 1, n)
            out.append(" " * (end - i))
            i = end
            continue
        if ch == "'" and re.match(r"'(\\.|[^\\'])'", line[i:]):
            m = re.match(r"'(\\.|[^\\'])'", line[i:])
            out.append(" " * m.end())
            i += m.end()
            continue
        out.append(ch)
        i += 1
    return "".join(out), in_block_comment


def scan_file(path, min_quals, include_allowed=False):
    hits = []
    in_block_comment = False
    in_use_item = False
    in_macro = False
    macro_depth = 0
    try:
        text = open(path, encoding="utf-8").read()
    except (OSError, UnicodeDecodeError) as err:
        print(f"warning: skipping {path}: {err}", file=sys.stderr)
        return hits
    for lineno, line in enumerate(text.splitlines(), start=1):
        cleaned, in_block_comment = strip_noise(line, in_block_comment)
        if in_use_item:
            if ";" in cleaned:
                in_use_item = False
            continue
        if USE_ITEM.match(cleaned):
            if ";" not in cleaned:
                in_use_item = True  # multiline use foo::{ ... };
            continue
        if OTHER_ITEM_SKIP.match(cleaned):
            continue
        if not include_allowed and ATTRIBUTE_LINE.match(cleaned):
            continue
        # Paths inside a macro_rules! body resolve at each expansion site, which
        # may be another crate with no such import — collapsing them to a bare
        # name silently breaks those callers. Report them, but flagged.
        if not in_macro and MACRO_RULES.match(cleaned):
            in_macro, macro_depth = True, 0
            seen_open = False
        if in_macro:
            macro_depth += cleaned.count("{") - cleaned.count("}")
            seen_open = seen_open or "{" in cleaned
            ends_here = seen_open and macro_depth <= 0
        # Waiver marker lives in the comment, which strip_noise blanked — so
        # look for it in the raw line.
        keep = "macro body" if in_macro else ("waiver" if KEEP_MARKER in line else None)
        for m in QUALIFIED_TYPE.finditer(cleaned):
            path = m.group(1)
            if path.count("::") < min_quals or not is_type_path(path):
                continue
            if not include_allowed and is_allowed(path):
                continue
            hits.append((lineno, m.start(1) + 1, path, keep))
        if in_macro and ends_here:
            in_macro = False
    return hits


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("paths", nargs="*", help="restrict scan to these paths (default: whole repo)")
    parser.add_argument("--list", action="store_true", help="print every occurrence as file:line:col: path")
    parser.add_argument(
        "--min-quals",
        type=int,
        default=1,
        metavar="N",
        help="minimum number of :: separators before the type (default 1; use 2 for deep paths only)",
    )
    parser.add_argument(
        "--all",
        action="store_true",
        help="include idiomatic uses too (fmt family, std::error::Error, attribute/derive lines)",
    )
    parser.add_argument(
        "--check",
        action="store_true",
        help="gate mode: exit 1 if any fixable hit exists (macro-body and"
        " keep-qualified-waived hits never fail)",
    )
    args = parser.parse_args()

    by_path = collections.Counter()
    occurrences = []
    fixes, keeps = [], []
    for file in tracked_rust_files(args.paths):
        for lineno, col, qualified, keep in scan_file(
            file, args.min_quals, include_allowed=args.all
        ):
            by_path[qualified] += 1
            # A macro-body hit is NOT safe to collapse to a bare name.
            note = f"  [{keep} — keep qualified]" if keep else ""
            loc = f"{file}:{lineno}:{col}: {qualified}"
            occurrences.append(loc + note)
            (keeps if keep else fixes).append(loc)

    if args.check:
        if fixes:
            print("qualified type paths found — import the type instead:", file=sys.stderr)
            for loc in fixes:
                print(f"  {loc}", file=sys.stderr)
            print(
                "\nFix: add `use <path>;` to the file's use block and write the bare"
                " type name.\nRules: paths inside macro_rules! bodies must STAY"
                " qualified (this check already skips them). If the type is only"
                " used under #[cfg(test)], gate the import with #[cfg(test)] or put"
                " it in the test module. Never import two same-named types bare —"
                " alias one, or waive the line with `// keep-qualified: <reason>`.",
                file=sys.stderr,
            )
        sys.exit(1 if fixes else 0)

    if args.list:
        for occ in occurrences:
            print(occ)
    else:
        for qualified, count in sorted(by_path.items(), key=lambda kv: (-kv[1], kv[0])):
            print(f"{count:6}  {qualified}")
    print(
        f"\n{sum(by_path.values())} occurrences of {len(by_path)} distinct qualified paths",
        file=sys.stderr,
    )
    if keeps:
        print(
            f"{len(keeps)} of them are deliberate keeps (macro_rules! bodies /"
            f" keep-qualified waivers) — see --list",
            file=sys.stderr,
        )


if __name__ == "__main__":
    main()
