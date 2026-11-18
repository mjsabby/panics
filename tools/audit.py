#!/usr/bin/env python3
"""Check clippy.toml against the standard library's own documentation.

Lists every stable item of `core`, `alloc` and `std` whose documentation has a "Panics"
section (see std_panics.py) and checks that each one is accounted for: banned in clippy.toml,
covered by an operator lint, or listed in tools/accounting.toml with the reason it is left out.
Also checks the other direction: an entry of clippy.toml that the documentation does not back
has to be explained in accounting.toml.

Run it after a toolchain update: new panicking functions show up as unaccounted.

    tools/audit.py            # check; exit status 1 if anything is unaccounted for
    tools/audit.py --update   # check, then rewrite docs/STD_PANICS.md
"""
from __future__ import annotations

import argparse
import re
import sys
import tomllib
from collections import defaultdict
from pathlib import Path

import std_panics
from std_panics import Item

ROOT = Path(__file__).resolve().parent.parent
SIGNED = ["i8", "i16", "i32", "i64", "i128", "isize"]
UNSIGNED = ["u8", "u16", "u32", "u64", "u128", "usize"]
PLACEHOLDERS = {"{int}": SIGNED + UNSIGNED, "{signed}": SIGNED, "{unsigned}": UNSIGNED}
# Non-panicking twins link back to the function they replace; that is not a sign of delegation.
TWIN = re.compile(r"^(checked_|try_|wrapping_|saturating_|overflowing_|unbounded_|get|is_)|_(checked|unchecked)$")


def pattern(item: str) -> re.Pattern:
    """`{int}::pow` and `impl Clone for RefCell<T>::*` as regular expressions."""
    rx = re.escape(item).replace(r"\*", ".*")
    for name, types in PLACEHOLDERS.items():
        rx = rx.replace(re.escape(name), "(?:" + "|".join(types) + ")")
    return re.compile(rx + "$")


def fold(ident: str) -> str:
    """One name for the twelve integer types and for the atomic types, to keep lists readable."""
    ident = re.sub(r"\b[iu](?:8|16|32|64|128|size)\b", "{int}", ident)
    return re.sub(r"\bAtomic(?:Bool|Ptr|[IU](?:8|16|32|64|128|size))\b", "Atomic*", ident)


def load():
    with open(ROOT / "clippy.toml", "rb") as f:
        config = tomllib.load(f)
    banned = {e["path"] if isinstance(e, dict) else e for key in ("disallowed-macros", "disallowed-methods")
              for e in config.get(key, [])}
    with open(ROOT / "tools" / "accounting.toml", "rb") as f:
        accounting = tomllib.load(f)
    return banned, accounting


def audit():
    banned, accounting = load()
    items = std_panics.all_items()
    documented = [it for it in items if it.panics and not it.unstable]
    problems: list[str] = []

    rules = [("lint", rule, pattern(i), i) for rule in accounting.get("lint", []) for i in rule["items"]]
    rules += [("skip", rule, pattern(i), i) for rule in accounting.get("skip", []) for i in rule["items"]]
    used = set()

    banned_items: list[Item] = []
    by_rule: dict[int, list[Item]] = defaultdict(list)
    for it in documented:
        if it.path in banned:
            banned_items.append(it)
            continue
        for kind, rule, rx, text in rules:
            if rx.match(it.ident):
                by_rule[id(rule)].append(it)
                used.add(text)
                break
        else:
            hint = f'    {{ path = "{it.path}", reason = "..." }},' if it.kind in ("method", "fn", "macro", "trait-method") else ""
            problems.append(f"not accounted for: {it.ident}\n      documented: {it.panics[:150]}\n  {hint}")
    for kind, rule, rx, text in rules:
        if text not in used:
            problems.append(f"accounting.toml: `{text}` matches no documented panic that is not banned")

    documented_paths = {it.path for it in documented}
    undocumented = accounting.get("undocumented", [])
    beyond: dict[int, list[str]] = defaultdict(list)
    for path in sorted(banned - documented_paths):
        for rule in undocumented:
            if any(pattern(i).match(path) for i in rule["items"]):
                beyond[id(rule)].append(path)
                break
        else:
            problems.append(f"clippy.toml bans {path}, which has no documented panic and no entry under [[undocumented]]")
    for rule in undocumented:
        for i in rule["items"]:
            if not any(pattern(i).match(p) for p in banned - documented_paths):
                problems.append(f"accounting.toml: [[undocumented]] `{i}` matches no entry of clippy.toml")

    # Functions that forward to a banned one usually say so with a link, and often nothing else.
    keys = {(p.rsplit("::", 2)[-2].replace("const_ptr", "pointer").replace("mut_ptr", "pointer"), p.rsplit("::", 1)[-1])
            for p in banned if p.count("::")}
    reviewed = [pattern(i) for rule in accounting.get("reviewed", []) for i in rule["items"]]
    for it in items:
        if it.unstable or it.panics or it.path in banned or it.kind in ("impl", "impl-method") or TWIN.search(it.name):
            continue
        if it.links & keys and not any(rx.match(it.path) for rx in reviewed):
            links = ", ".join("::".join(link) for link in sorted(it.links & keys))
            problems.append(f"{it.path} documents no panic but links to the banned {links}; ban it or list it under [[reviewed]]")

    return problems, documented, banned, banned_items, by_rule, beyond, accounting


# ── docs/STD_PANICS.md ───────────────────────────────────────────────────────────────────────

def listing(names: list[str]) -> str:
    """`a`, `b` ×12, ... with integer and atomic types folded."""
    counts: dict[str, int] = {}
    for name in names:
        counts[fold(name)] = counts.get(fold(name), 0) + 1
    return ", ".join(f"`{name}`" + (f" ×{n}" if n > 1 else "") for name, n in counts.items())


def write_report(documented, banned, banned_items, by_rule, beyond, accounting) -> None:
    lint_rules = accounting.get("lint", [])
    skip_rules = accounting.get("skip", [])
    n_lint = sum(len(by_rule[id(r)]) for r in lint_rules)
    n_skip = sum(len(by_rule[id(r)]) for r in skip_rules)
    n_beyond = sum(len(v) for v in beyond.values())
    out = [
        "# Every documented panic in the standard library, and what happens to it",
        "",
        f"Generated by `tools/audit.py --update` with rustc {std_panics.rust_version()}. Do not edit by hand;",
        "the reasons come from `tools/accounting.toml`.",
        "",
        f"The documentation of `core`, `alloc` and `std` has a \"Panics\" section on {len(documented)} stable items",
        "(functions, methods, macros and trait implementations; every integer type counts separately).",
        "",
        "| What happens to it | Items |",
        "|---|---:|",
        f"| Banned in `clippy.toml` | {len(banned_items)} |",
        f"| Covered by an operator lint | {n_lint} |",
        f"| Not banned, for the reasons below | {n_skip} |",
        "",
        f"`clippy.toml` has {len(banned)} entries. {len({it.path for it in banned_items})} of them cover those {len(banned_items)} documented items (one entry",
        "can cover several, as `NonZero::from_str_radix` does for every integer type). The other",
        f"{n_beyond} panic although their documentation has no \"Panics\" section.",
        "",
        "## Not banned",
        "",
    ]
    for rule in skip_rules:
        out += [f"### {rule['title']}", "", " ".join(rule["reason"].split()), ""]
        out += ["| Item | Documented condition |", "|---|---|"]
        seen: dict[str, str] = {}
        counts: dict[str, int] = defaultdict(int)
        for it in by_rule[id(rule)]:
            name = fold(it.ident)
            counts[name] += 1
            seen.setdefault(name, it.panics)
        for name, text in seen.items():
            text = text if len(text) <= 220 else text[:217].rsplit(" ", 1)[0] + "..."
            times = f" ×{counts[name]}" if counts[name] > 1 else ""
            out.append(f"| `{name}`{times} | {text.replace('|', chr(92) + '|')} |")
        out.append("")

    out += ["## Covered by an operator lint", "", "| Lint | Items | Note |", "|---|---|---|"]
    for rule in lint_rules:
        out.append(f"| `{rule['lint']}` | {listing([it.ident for it in by_rule[id(rule)]])} | {rule['note']} |")

    out += ["", "## Banned without a \"Panics\" section in the documentation", "", "| Entries | Why they are in clippy.toml |", "|---|---|"]
    for rule in accounting.get("undocumented", []):
        out.append(f"| {listing(beyond[id(rule)])} | {rule['note']} |")

    out += [
        "", "## Banned", "",
        "The entries themselves, with the replacement for each, are in [`clippy.toml`](../clippy.toml);",
        "[COVERAGE.md](COVERAGE.md) shows each one panicking.",
    ]
    (ROOT / "docs").mkdir(exist_ok=True)
    (ROOT / "docs" / "STD_PANICS.md").write_text("\n".join(out) + "\n", encoding="utf-8")


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--update", action="store_true", help="rewrite docs/STD_PANICS.md after a successful check")
    args = ap.parse_args()

    problems, documented, banned, banned_items, by_rule, beyond, accounting = audit()
    n_lint = sum(len(by_rule[id(r)]) for r in accounting.get("lint", []))
    n_skip = sum(len(by_rule[id(r)]) for r in accounting.get("skip", []))
    print(f"rustc {std_panics.rust_version()}: {len(documented)} stable items document a panic")
    print(f"  banned in clippy.toml:        {len(banned_items)}")
    print(f"  covered by an operator lint:  {n_lint}")
    print(f"  not banned, with a reason:    {n_skip}")
    print(f"clippy.toml: {len(banned)} entries, {sum(len(v) for v in beyond.values())} of them beyond the documentation")
    if problems:
        print(f"\n{len(problems)} problem(s):")
        for problem in problems:
            print(f"  - {problem}")
        sys.exit(1)
    print("everything is accounted for")
    if args.update:
        write_report(documented, banned, banned_items, by_rule, beyond, accounting)
        print("wrote docs/STD_PANICS.md")


if __name__ == "__main__":
    main()
