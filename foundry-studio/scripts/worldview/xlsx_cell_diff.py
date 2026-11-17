#!/usr/bin/env python3
"""Cell-by-cell comparison of two workbooks under a NAMED exclusion list.

Harness A of the worldview parity plan: master's desvis and the branch's
`workbook` command must produce cell-identical workbooks for the same run,
except the cells that legitimately differ between two processes — the wall
clock, the timestamp, the hostname, the commit — which are excluded BY NAME.

What a cell is compared on: its value AND its number format, because a book
showing `50.00%` and a book showing `0.5` do not say the same thing to a
reader even though the number behind them is one number.

What names a cell: its row label (column A) and its column header (row 1),
read from BOTH workbooks. A coordinate is excluded only when both sides name
it with the same provenance word, so a renamed label cannot quietly widen the
exclusion; the labels and headers are also compared in their own right, so a
rename is itself a difference. Every excluded name must match at least one
cell. Exit 1 on any other difference, on a sheet present on one side only, or
on an unmatched name.
"""
import sys
from pathlib import Path

import openpyxl

EXCLUDED_NAMES = ["wall clock", "wall_clock", "timestamp", "hostname", "commit"]
MAX_REPORTED = 60


def load(path):
    return openpyxl.load_workbook(path, data_only=False, read_only=False)


def named(text):
    """The provenance word this label or header carries, if any."""
    if text is None:
        return None
    lowered = str(text).lower()
    for name in EXCLUDED_NAMES:
        if name in lowered:
            return name
    return None


def excluded_reason(a_label, b_label, a_header, b_header):
    """The provenance word both sides give this coordinate, if they agree."""
    for a_text, b_text in ((a_label, b_label), (a_header, b_header)):
        a_name = named(a_text)
        if a_name is not None and a_name == named(b_text):
            return a_name
    return None


def names_something(a_text, b_text):
    """Whether this label or header is a name at all rather than a value."""
    return isinstance(a_text, str) or isinstance(b_text, str)


def cell_state(sheet, row, col):
    cell = sheet.cell(row=row, column=col)
    return cell.value, cell.number_format


def main(a_path, b_path):
    a, b = load(a_path), load(b_path)
    problems = []
    names_matched = {name: 0 for name in EXCLUDED_NAMES}
    compared = excluded = 0
    a_sheets, b_sheets = list(a.sheetnames), list(b.sheetnames)
    if a_sheets != b_sheets:
        problems.append(f"sheet lists differ: {a_sheets} vs {b_sheets}")
    for name in [s for s in a_sheets if s in b_sheets]:
        sa, sb = a[name], b[name]
        max_row = max(sa.max_row, sb.max_row)
        max_col = max(sa.max_column, sb.max_column)
        labels = {r: (sa.cell(row=r, column=1).value, sb.cell(row=r, column=1).value) for r in range(1, max_row + 1)}
        headers = {c: (sa.cell(row=1, column=c).value, sb.cell(row=1, column=c).value) for c in range(1, max_col + 1)}
        # The names themselves, compared before anything is excluded by them.
        # Only text names anything: column A of a sheet with no label column,
        # and row 1 of a sheet whose first row is data, hold values, and those
        # are the value loop's business.
        for r, (va, vb) in labels.items():
            if names_something(va, vb) and va != vb:
                problems.append(f"{name}!A{r}: row label {va!r} vs {vb!r}")
        for c, (va, vb) in headers.items():
            if names_something(va, vb) and va != vb:
                problems.append(f"{name}!{sa.cell(row=1, column=c).coordinate}: column header {va!r} vs {vb!r}")
        for r in range(1, max_row + 1):
            a_label, b_label = labels[r]
            for c in range(1, max_col + 1):
                a_header, b_header = headers[c]
                reason = excluded_reason(a_label, b_label, a_header, b_header)
                if reason is not None:
                    excluded += 1
                    names_matched[reason] += 1
                    continue
                va, fa = cell_state(sa, r, c)
                vb, fb = cell_state(sb, r, c)
                compared += 1
                if va != vb:
                    problems.append(f"{name}!{sa.cell(row=r, column=c).coordinate}: {va!r} vs {vb!r}")
                elif fa != fb:
                    problems.append(f"{name}!{sa.cell(row=r, column=c).coordinate}: format {fa!r} vs {fb!r}")
    unmatched = [n for n, k in names_matched.items() if k == 0]
    print(f"compared {compared} cells, excluded {excluded} ({names_matched})")
    if unmatched:
        problems.append(f"excluded names that matched no cell: {unmatched}")
    if problems:
        print(f"DIFFERENT: {len(problems)} problem(s)")
        for p in problems[:MAX_REPORTED]:
            print("  " + p)
        if len(problems) > MAX_REPORTED:
            print(f"  … and {len(problems) - MAX_REPORTED} more")
        return 1
    print("IDENTICAL under the exclusion list")
    return 0


if __name__ == "__main__":
    if len(sys.argv) != 3:
        print("usage: xlsx_cell_diff.py <a.xlsx> <b.xlsx>", file=sys.stderr)
        sys.exit(2)
    sys.exit(main(Path(sys.argv[1]), Path(sys.argv[2])))
