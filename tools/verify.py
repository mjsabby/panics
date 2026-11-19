#!/usr/bin/env python3
"""Prove the lint configuration with the probe crate.

Runs clippy on `probe`, runs the probe's harness in a debug and a release build, and checks:

  * every entry in clippy.toml has a snippet that panics at run time and that clippy flags
    as a use of exactly that entry;
  * every operator snippet panics and is flagged by the operator lint it names;
  * every known gap still panics and is still flagged by nothing;
  * `example`, the crate of compliant code, passes clippy with warnings denied, and its tests pass;
  * a `no_std` crate gets the `core` entries and no complaints about the others;
  * the commented-out entries at the end of clippy.toml name things that exist, and the
    commented-out lint in Cargo.toml flags the two gaps it is offered for.

    tools/verify.py            # check; exit status 1 if anything is off
    tools/verify.py --update   # check, then rewrite docs/COVERAGE.md
"""
from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
import sys
import tomllib
from collections import defaultdict
from dataclasses import dataclass
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
LINTS = ("disallowed_macros", "disallowed_methods", "indexing_slicing", "string_slice", "arithmetic_side_effects")
SNIPPET_MODULES = ("bans", "lints", "gaps")
NOT_SNIPPETS = {"bans::tls_register"}  # public only because the harness needs it
INT = re.compile(r"^[iu](?:8|16|32|64|128|size)$")
DISALLOWED = re.compile(r"use of a disallowed (?:method|macro) `([^`]+)`")


@dataclass
class Snippet:
    function: str  # module::name
    file: str
    first_line: int
    last_line: int
    doc: str
    source: str


@dataclass
class Diagnostic:
    lint: str
    path: str  # the clippy.toml entry, for the two ban lints
    file: str
    line: int  # where the outermost macro call is, i.e. a line of the snippet


@dataclass
class Record:
    kind: str
    id: str
    function: str
    panicked: bool
    message: str


def run(cmd: list[str], env: dict | None = None, cwd: Path = ROOT) -> subprocess.CompletedProcess:
    return subprocess.run(cmd, cwd=cwd, env={**os.environ, **(env or {})}, capture_output=True, text=True)


def tool_versions() -> str:
    rustc = run(["rustc", "--version"]).stdout.split()[1]
    clippy = run(["cargo", "clippy", "--version"]).stdout.split()[1]
    return f"rustc {rustc}, clippy {clippy}"


def ban_list() -> dict[str, str]:
    """Entries of clippy.toml: path -> 'macro' or 'method'."""
    with open(ROOT / "clippy.toml", "rb") as f:
        config = tomllib.load(f)
    entries = {}
    for key, kind in (("disallowed-macros", "macro"), ("disallowed-methods", "method")):
        for entry in config.get(key, []):
            entries[entry["path"] if isinstance(entry, dict) else entry] = kind
    return entries


def snippets() -> dict[str, Snippet]:
    """Top-level functions of the snippet modules, with their line ranges and doc comments."""
    found = {}
    for module in SNIPPET_MODULES:
        file = f"{module}.rs"
        lines = (ROOT / "probe" / "src" / file).read_text(encoding="utf-8").splitlines()
        for i, line in enumerate(lines):
            m = re.match(r"pub (?:const )?fn (\w+)", line)
            if not m:
                continue
            depth, j, opened = 0, i, False
            while True:  # the function ends where its braces balance
                depth += lines[j].count("{") - lines[j].count("}")
                opened = opened or "{" in lines[j]
                if opened and depth <= 0:
                    break
                j += 1
            doc = []
            k = i - 1
            while k >= 0 and lines[k].startswith("///"):
                doc.insert(0, lines[k][3:].strip())
                k -= 1
            function = f"{module}::{m.group(1)}"
            found[function] = Snippet(function, file, i + 1, j + 1, " ".join(doc), "\n".join(lines[i: j + 1]))
    return found


def int_snippet_sources() -> dict[str, str]:
    """Body of each generated integer snippet in ints.rs: method -> expression."""
    text = (ROOT / "probe" / "src" / "ints.rs").read_text(encoding="utf-8")
    return {m.group(1): m.group(2).strip().replace("<$t>", "T")
            for m in re.finditer(r"pub fn (\w+)\([^)]*\)[^{]*\{(.*)\}", text)}


def clippy(package: str, extra: list[str] | None = None, cwd: Path = ROOT, manifest: Path | None = None):
    """Diagnostics of one package's library, and messages that are not about code (config problems)."""
    cmd = ["cargo", "clippy", "--quiet", "--message-format=json"]
    cmd += ["--manifest-path", str(manifest)] if manifest else ["-p", package, "--lib"]
    result = run(cmd + (extra or []), cwd=cwd)
    diagnostics, other = [], []
    for raw in result.stdout.splitlines():
        if not raw.startswith("{"):
            continue
        obj = json.loads(raw)
        if obj.get("reason") != "compiler-message" or obj["target"]["name"].replace("-", "_") != package.replace("-", "_"):
            continue
        message = obj["message"]
        code = (message.get("code") or {}).get("code") or ""
        primary = [s for s in message["spans"] if s.get("is_primary")]
        if not code or not primary:
            if not message["message"].startswith(("aborting due to", "could not compile")):
                other.append(f'{message["level"]}: {message["message"]}')
            continue
        span = primary[0]
        while span.get("expansion"):
            span = span["expansion"]["span"]
        path = DISALLOWED.search(message["message"])
        diagnostics.append(Diagnostic(code.removeprefix("clippy::"), path.group(1) if path else "",
                                      os.path.basename(span["file_name"]), span["line_start"]))
    return diagnostics, other


def harness(release: bool) -> list[Record]:
    cmd = ["cargo", "run", "--quiet", "-p", "probe"] + (["--release"] if release else [])
    result = run(cmd)
    if result.returncode != 0:
        sys.exit(f"the probe harness failed:\n{result.stderr}")
    records = []
    for line in result.stdout.splitlines():
        kind, ident, function, outcome, message = line.split("\t")
        records.append(Record(kind, ident, function, outcome == "panic", message))
    return records


def check_no_std(failures: list[str]) -> None:
    """clippy.toml in a crate that links neither alloc nor std."""
    crate = ROOT / "target" / "verify-no-std"
    (crate / "src").mkdir(parents=True, exist_ok=True)
    (crate / "Cargo.toml").write_text(
        '[package]\nname = "verify-no-std"\nversion = "0.0.0"\nedition = "2024"\n\n[workspace]\n')
    (crate / "src" / "lib.rs").write_text(
        "#![no_std]\n"
        "pub fn a(o: Option<u32>) -> u32 { o.unwrap() }\n"
        "pub fn b(v: &[u8], n: usize) -> usize { v.split_at(n).0.len() }\n"
        'pub fn c() { panic!("x") }\n')
    diagnostics, other = clippy("verify-no-std", ["--", "-D", "clippy::disallowed_methods", "-D", "clippy::disallowed_macros"],
                                cwd=crate, manifest=crate / "Cargo.toml")
    got = sorted(d.path for d in diagnostics)
    want = ["core::option::Option::unwrap", "core::panic", "slice::split_at"]
    if got != want:
        failures.append(f"no_std crate: expected clippy to flag {want}, got {got}")
    if other:
        failures.append(f"no_std crate: clippy complained about the configuration: {other}")


def check_optional_block(failures: list[str]) -> None:
    """The commented-out entries at the end of clippy.toml: enabled, every path must resolve."""
    text = (ROOT / "clippy.toml").read_text(encoding="utf-8")
    enabled = re.sub(r"^    # (\{ path = )", r"    \1", text, flags=re.M)
    optional = enabled.count("{ path = ") - text.count("\n    { path = ")
    crate = ROOT / "target" / "verify-optional"
    (crate / "src").mkdir(parents=True, exist_ok=True)
    (crate / "clippy.toml").write_text(enabled, encoding="utf-8")
    (crate / "Cargo.toml").write_text(
        '[package]\nname = "verify-optional"\nversion = "0.0.0"\nedition = "2024"\n\n[workspace]\n')
    (crate / "src" / "lib.rs").write_text("pub fn a(n: usize) -> Vec<u8> { Vec::with_capacity(n) }\n")
    diagnostics, other = clippy("verify-optional", ["--", "-D", "clippy::disallowed_methods"],
                                cwd=crate, manifest=crate / "Cargo.toml")
    if optional == 0:
        failures.append("optional block of clippy.toml: no commented-out entries found")
    if [d.path for d in diagnostics] != ["alloc::vec::Vec::with_capacity"]:
        failures.append(f"optional block of clippy.toml: enabling it does not flag Vec::with_capacity ({diagnostics})")
    if other:
        failures.append(f"optional block of clippy.toml: clippy complained when it was enabled: {other}")


def check_optional_lint(failures: list[str], known: dict[str, Snippet]) -> None:
    """The lint that is commented out in Cargo.toml has to close the two gaps it is offered for."""
    lint = "integer_division_remainder_used"
    if f'# {lint} = "deny"' not in (ROOT / "Cargo.toml").read_text(encoding="utf-8"):
        failures.append(f"Cargo.toml: the optional line for {lint} is missing")
    diagnostics, _ = clippy("probe", ["--", "-W", f"clippy::{lint}"])
    for function in ("gaps::divide_by_minus_one", "gaps::arithmetic_in_static"):
        s = known[function]
        if not any(d.lint == lint and d.file == s.file and s.first_line <= d.line <= s.last_line for d in diagnostics):
            failures.append(f"optional lint {lint}: does not flag {function}")


def check_example(failures: list[str]) -> None:
    lint = run(["cargo", "clippy", "--quiet", "-p", "example", "--all-targets", "--", "-D", "warnings"])
    if lint.returncode != 0:
        failures.append("example: clippy is not clean:\n" + lint.stderr.strip())
    test = run(["cargo", "test", "--quiet", "-p", "example"])
    if test.returncode != 0:
        failures.append("example: tests fail:\n" + test.stdout.strip())


def verify():
    failures: list[str] = []
    bans = ban_list()
    known = snippets()
    diagnostics, other = clippy("probe")
    debug, release = harness(False), harness(True)
    if other:
        failures.append(f"clippy complained about the configuration: {other}")

    flagged_paths = {d.path for d in diagnostics if d.path}

    def lints_in(snippet: Snippet) -> list[Diagnostic]:
        return [d for d in diagnostics if d.file == snippet.file and snippet.first_line <= d.line <= snippet.last_line]

    release_panics = {(r.kind, r.id, r.function): r.panicked for r in release}
    by_entry: dict[str, list[Record]] = defaultdict(list)
    seen_functions = set()
    for r in debug:
        seen_functions.add(r.function)
        where = f"{r.kind} {r.id or r.function} ({r.function})"
        if not r.panicked:
            failures.append(f"{where}: did not panic")
        generated = r.function.startswith("ints::")
        snippet = known.get(r.function)
        if not generated and snippet is None:
            failures.append(f"{where}: no such snippet in the probe")
            continue
        found = [] if generated else lints_in(snippet)
        if r.kind == "ban":
            by_entry[r.id].append(r)
            if r.id not in bans:
                failures.append(f"{where}: not an entry of clippy.toml")
            elif r.id not in flagged_paths:
                failures.append(f"{where}: clippy never reports a use of this entry")
            elif not generated and not any(d.path == r.id for d in found):
                failures.append(f"{where}: clippy does not flag this snippet as a use of the entry")
        elif r.kind == "lint":
            if not any(d.lint == r.id for d in found):
                failures.append(f"{where}: the lint does not flag this snippet")
        elif r.kind == "gap":
            hits = sorted({d.lint for d in found if d.lint in LINTS})
            if hits:
                failures.append(f"{where}: no longer a gap, {hits} flags it now; move it and update the docs")
    for entry in bans:
        if entry not in by_entry:
            failures.append(f"clippy.toml entry {entry}: the probe has no snippet for it")
    for function in known:
        if function not in seen_functions and function not in NOT_SNIPPETS:
            failures.append(f"snippet {function}: the harness never runs it")
    for d in diagnostics:
        if d.file in {f"{m}.rs" for m in SNIPPET_MODULES} and not any(
                s.file == d.file and s.first_line <= d.line <= s.last_line for s in known.values()):
            failures.append(f"{d.file}:{d.line}: {d.lint} fires outside any snippet")

    check_example(failures)
    check_no_std(failures)
    check_optional_block(failures)
    check_optional_lint(failures, known)
    return failures, bans, known, debug, release_panics


# ── docs/COVERAGE.md ─────────────────────────────────────────────────────────────────────────

def cell(text: str) -> str:
    return text.replace("|", "\\|").replace("\n", " ")


def code(text: str) -> str:
    return f"`{cell(text)}`" if text else ""


def body_of(source: str) -> str:
    """The interesting part of a one-line snippet: what is between the outer braces."""
    m = re.search(r"\{(.*)\}\s*$", source, re.S)
    return " ".join(m.group(1).split()) if m else source


def whole(source: str) -> str:
    """A snippet with its signature, for the tables where the parameter types matter."""
    return " ".join(source.split()).removeprefix("pub ")


def write_coverage(bans, known, debug, release_panics) -> None:
    def when(r: Record) -> str:
        return "every build" if release_panics.get((r.kind, r.id, r.function)) else "overflow checks or debug assertions on"

    ban_records = [r for r in debug if r.kind == "ban"]
    lint_records = [r for r in debug if r.kind == "lint"]
    gap_records = [r for r in debug if r.kind == "gap"]
    debug_only = sum(1 for r in ban_records + lint_records + gap_records if when(r) != "every build")
    macros = sum(1 for kind in bans.values() if kind == "macro")
    int_sources = int_snippet_sources()

    out = [
        "# What the probe shows",
        "",
        f"Generated by `tools/verify.py --update` with {tool_versions()}. Do not edit by hand.",
        "",
        "Every snippet below was run and panicked. \"Flagged\" means clippy reports it under the",
        "configuration in this repository.",
        "",
        "| | Snippets | Flagged |",
        "|---|---:|---:|",
        f"| Entries of `clippy.toml` ({macros} macros, {len(bans) - macros} functions) | {len(ban_records)} | {len(ban_records)} |",
        f"| Operators, covered by `indexing_slicing`, `string_slice`, `arithmetic_side_effects` | {len(lint_records)} | {len(lint_records)} |",
        f"| Known gaps | {len(gap_records)} | 0 |",
        "",
        f"{debug_only} of the {len(debug)} snippets panic only when overflow checks or debug assertions are on,",
        "which is the default for debug builds. The rest panic in release builds too.",
        "",
        "## Known gaps",
        "",
        "These panic, and nothing in the configuration flags them.",
        "",
        "| Snippet | Why it gets through | Panic message | Panics in |",
        "|---|---|---|---|",
    ]
    for r in gap_records:
        s = known[r.function]
        out.append(f"| {code(whole(s.source))} | {cell(s.doc)} | {cell(r.message)} | {when(r)} |")

    out += ["", "## Operators", "", "| Lint | Snippet | Panic message | Panics in |", "|---|---|---|---|"]
    per_type: dict[tuple, list[str]] = {}
    for r in lint_records:
        name = r.function.split("::")[1]
        ty, _, method = name.partition("_")
        if INT.match(ty):  # the same method on every integer type: one row
            per_type.setdefault((r.id, method, r.message, when(r)), []).append(ty)
        else:
            out.append(f"| `{r.id}` | {code(whole(known[r.function].source))} | {cell(r.message)} | {when(r)} |")
    for (lint, method, message, panics_in), types in per_type.items():
        on = "all 12 integer types" if len(types) == 12 else ", ".join(types)
        out.append(f"| `{lint}` | `a.{method}(b)` on {on} | {cell(message)} | {panics_in} |")

    out += [
        "", "## Entries of clippy.toml", "",
        "One row per snippet. An entry with several rows is reached in several ways, for example through a macro.",
        "Integer methods are listed once, with the types they were run on.",
        "", "| Entry | Snippet | Panic message | Panics in |", "|---|---|---|---|",
    ]
    folded: dict[tuple, list[str]] = {}
    for r in ban_records:
        if r.function.startswith("ints::"):
            ty, method = r.id.split("::")
            folded.setdefault((method, r.message, when(r)), []).append(ty)
        else:
            out.append(f"| `{r.id}` | {code(body_of(known[r.function].source))} | {cell(r.message)} | {when(r)} |")
    for (method, message, panics_in), types in folded.items():
        signed = [t for t in types if t.startswith("i")]
        unsigned = [t for t in types if t.startswith("u")]
        on = "all 12 integer types" if len(types) == 12 else (
            "the 6 signed integer types" if len(signed) == 6 and not unsigned else
            "the 6 unsigned integer types" if len(unsigned) == 6 and not signed else ", ".join(types))
        out.append(f"| `{method}` on {on} | {code(int_sources.get(method, ''))} | {cell(message)} | {panics_in} |")

    (ROOT / "docs").mkdir(exist_ok=True)
    (ROOT / "docs" / "COVERAGE.md").write_text("\n".join(out) + "\n", encoding="utf-8")


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--update", action="store_true", help="rewrite docs/COVERAGE.md after a successful check")
    args = ap.parse_args()

    failures, bans, known, debug, release_panics = verify()
    kinds = defaultdict(int)
    for r in debug:
        kinds[r.kind] += 1
    debug_only = sum(1 for r in debug if not release_panics.get((r.kind, r.id, r.function)))
    print(f"{tool_versions()}")
    print(f"clippy.toml: {len(bans)} entries, {kinds['ban']} snippets")
    print(f"operators:   {kinds['lint']} snippets")
    print(f"known gaps:  {kinds['gap']} snippets")
    print(f"{debug_only} of {len(debug)} snippets panic only with overflow checks or debug assertions on")
    if failures:
        print(f"\n{len(failures)} problem(s):")
        for failure in failures:
            print(f"  - {failure}")
        sys.exit(1)
    print("all checks pass")
    if args.update:
        write_coverage(bans, known, debug, release_panics)
        print("wrote docs/COVERAGE.md")


if __name__ == "__main__":
    main()
