#!/usr/bin/env python3
"""Package the finished matrix reports and their linked evidence for offline sharing.

Only packaged copies have their local evidence links rewritten. Source reports
and audit files stay unchanged; recordings are not needed to view the reports.
"""
# ARCHIVE TOOL: the DESPROF v4 container this parses was retired with the
# `.worldview` format. It still opens recordings made by binaries from before
# that change; nothing writes the format any more.
import argparse
import hashlib
from html import escape
from html.parser import HTMLParser
import json
from pathlib import Path
import tempfile
from urllib.parse import unquote, urlsplit, urlunsplit
from zipfile import ZIP_DEFLATED, ZipFile

REPO = Path(__file__).resolve().parents[2]
DOCUMENT_NAMES = ('report.html', 'variants.html')


class References(HTMLParser):
    def __init__(self, text):
        super().__init__()
        self.links = []
        self.ids = set()
        self.feed(text)
        self.close()

    def handle_starttag(self, tag, attrs):
        for attr, value in attrs:
            if value is not None and attr in ('href', 'src'):
                self.links.append((attr, value))
            if attr == 'id' or (tag == 'a' and attr == 'name'):
                self.ids.add(value)


def sha256(data):
    return hashlib.sha256(data).hexdigest()


def replace_attribute(text, attr, value, replacement):
    # The report generators quote every URL. Refuse unexpected markup rather
    # than silently shipping a link that still points at the author's disk.
    for quote in ('"', "'"):
        before = f'{attr}={quote}{escape(value, quote=True)}{quote}'
        if before in text:
            after = f'{attr}={quote}{escape(replacement, quote=True)}{quote}'
            return text.replace(before, after)
    raise ValueError(f'Cannot rewrite {attr}={value!r}')


def validate(files):
    documents = {name: References(files[name].decode()) for name in DOCUMENT_NAMES}
    count = 0
    for name, document in documents.items():
        for attr, value in document.links:
            ref = urlsplit(value)
            if ref.scheme == 'data':
                continue
            if ref.scheme or ref.netloc or ref.query:
                raise ValueError(f'{name}: non-portable {attr}={value!r}')
            target = unquote(ref.path) or name
            if target not in files:
                raise ValueError(f'{name}: missing package target {target!r}')
            if ref.fragment and (
                target not in documents or unquote(ref.fragment) not in documents[target].ids
            ):
                raise ValueError(f'{name}: missing anchor {value!r}')
            count += 1
    for name, data in files.items():
        if name.endswith('.json'):
            json.loads(data)
    return count


def package(base, documents, output):
    base, documents, output = base.resolve(), documents.resolve(), output.resolve()
    sources = {name: (documents / name).read_bytes() for name in DOCUMENT_NAMES}
    files = {}
    rewritten = []
    link_root = None
    for name, source in sources.items():
        text = source.decode()
        links = dict.fromkeys(References(text).links)
        for attr, value in links:
            ref = urlsplit(value)
            path = Path(unquote(ref.path))
            if ref.scheme == 'file' or (not ref.scheme and path.is_absolute()):
                if ref.netloc not in ('', 'localhost'):
                    raise ValueError(f'{name}: unsupported file host in {value!r}')
                # Matrix evidence lives at <archive>/<case>/<file>. Resolve it
                # under --base so the bundle can be rebuilt on another machine.
                if link_root is None:
                    link_root = path.parent.parent
                relative = path.relative_to(link_root)
                path = (base / relative).resolve()
                path.relative_to(base)
                if path.suffix not in ('.json', '.log'):
                    raise ValueError(f'{name}: unexpected evidence type {path}')
                packaged_path = 'evidence/' + relative.as_posix()
                files[packaged_path] = path.read_bytes()
                replacement = urlunsplit(('', '', packaged_path, ref.query, ref.fragment))
                text = replace_attribute(text, attr, value, replacement)
                rewritten.append({'document': name, 'original': value, 'packaged': replacement})
        if name == 'variants.html':
            text = text.replace('Original report SHA-256 (unchanged):',
                                'Original report source SHA-256 (before portable link rewriting):')
        files[name] = text.encode()

    files['README.txt'] = (
        'Walmart throughput reports — portable report bundle\n\n'
        '1. Unzip the entire archive, preserving its folders.\n'
        '2. Open variants.html in a browser. Its charts and CSV/JSON downloads work offline.\n'
        '3. Use "Read the original report" for the matrix discussion and spatial figures.\n'
        '   Its evidence links open the included JSON audits and preflight logs.\n\n'
        'Keep both HTML files and the evidence folder together. JavaScript is needed\n'
        'for the variant charts. No server, installation or simulation run is needed.\n\n'
        'Only packaged HTML copies have local links/provenance wording adjusted.\n'
        'Evidence files are byte-for-byte copies. Paths inside their data record the\n'
        'original experiment; they are provenance, not dependencies for viewing.\n'
        'package-manifest.json records the original and packaged file hashes.\n\n'
        'This is a report/evidence bundle, not a simulation reproduction archive.\n'
        'Large .desprofile recordings, binaries and source fixtures are not included.\n'
    ).encode()
    link_count = validate(files)
    manifest = {
        'format_version': 1,
        'source_documents': {name: {'sha256': sha256(data)} for name, data in sources.items()},
        'original_evidence_root': str(link_root) if link_root is not None else None,
        'rewritten_links': rewritten,
        'files': {name: {'bytes': len(data), 'sha256': sha256(data)}
                  for name, data in sorted(files.items())},
        'checked_html_links': link_count,
        'scope': 'Offline reports and linked evidence; source recordings are excluded.',
    }
    files['package-manifest.json'] = (json.dumps(manifest, indent=2) + '\n').encode()
    if output in [documents / name for name in DOCUMENT_NAMES] or output.suffix != '.zip':
        raise ValueError('Output must be a .zip file, separate from the source reports')
    output.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(dir=output.parent) as temporary:
        archive = Path(temporary) / 'reports.zip'
        with ZipFile(archive, 'w', compression=ZIP_DEFLATED, compresslevel=6) as bundle:
            for name, data in sorted(files.items()):
                bundle.writestr(name, data)
        with ZipFile(archive) as bundle:
            if bundle.testzip() is not None:
                raise ValueError('Archive integrity check failed')
        for name, source in sources.items():
            if (documents / name).read_bytes() != source:
                raise ValueError(f'Source report changed during packaging: {name}')
        archive.replace(output)
    print(json.dumps({'output': str(output), 'bytes': output.stat().st_size,
                      'files': len(files), 'evidence_files': len(files) - 4,
                      'checked_html_links': link_count}, indent=2))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--base', type=Path, default=REPO.parent / 'notes/walmart-matrix-2026-09-14',
                        help='Directory containing the linked per-case evidence files')
    parser.add_argument('--documents', type=Path,
                        default=REPO / 'docs/easyloop/overnight-2026-09-14',
                        help='Directory containing report.html and variants.html')
    parser.add_argument('--output', type=Path, required=True, help='Destination ZIP file')
    args = parser.parse_args()
    package(args.base, args.documents, args.output)


if __name__ == '__main__':
    main()
