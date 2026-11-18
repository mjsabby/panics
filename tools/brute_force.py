#!/usr/bin/env python3
"""Find panics the documentation does not mention.

`audit.py` relies on the standard library documenting its panics. For the largest family, the
methods of the integer and float types and of `char`, this script does not: it calls every
stable, safe method with boundary values (`MIN`, `MAX`, 0, 1, -1, NaN, ...) in a debug build
and checks that each one that panics is banned in clippy.toml or covered by an operator lint
according to tools/accounting.toml. That is how the overflow in `pow` was found.

    tools/brute_force.py     # takes a minute; exit status 1 if a panicking method is not accounted for
"""
from __future__ import annotations

import os
import re
import subprocess
import sys
import tomllib
from pathlib import Path

import std_panics
from audit import SIGNED, UNSIGNED, pattern

ROOT = Path(__file__).resolve().parent.parent
INTS = SIGNED + UNSIGNED
FLOATS = ["f32", "f64"]
OWNERS = INTS + FLOATS + ["char", "bool"]
SIGNATURE = re.compile(r"pub (?:const )?fn \w+(?:<[^>]*>)?\((.*?)\)(?: -> .*)?$")


def values(ty: str, owner: str) -> str | None:
    """A Rust array expression with boundary values of the type, or None if it is not supported."""
    ty = owner if ty == "Self" else ty
    if ty in INTS:
        return f"[{ty}::MIN, {ty}::MAX, 0, 1, 2, 7{', -1, -7' if ty in SIGNED else ''}]"
    if ty in FLOATS:
        return f"[{ty}::NAN, {ty}::INFINITY, {ty}::NEG_INFINITY, {ty}::MAX, {ty}::MIN, 0.0, -0.0, 1.5, -2.5]"
    if ty == "bool":
        return "[true, false]"
    if ty == "char":
        return "['a', '9', 'é', char::MAX]"
    if ty == "&str":
        return '["", "1", "-", "zz"]'
    if m := re.fullmatch(r"\[u8; (\d+)\]", ty):
        return f"[[0_u8; {m.group(1)}], [0xff; {m.group(1)}]]"
    if (m := re.fullmatch(r"NonZero<(\w+)>", ty)) and m.group(1) in INTS:
        return f"[std::num::NonZero::<{m.group(1)}>::MIN, std::num::NonZero::<{m.group(1)}>::MAX]"
    return None


def call_block(owner: str, name: str, signature: str) -> str | None:
    """Rust code that calls one method with every combination of boundary values."""
    m = SIGNATURE.match(signature)
    if not m:
        return None
    arrays, args, receiver = [], [], False
    for i, param in enumerate(p.strip() for p in re.split(r",(?![^\[<]*[\]>])", m.group(1)) if p.strip()):
        if param in ("self", "mut self"):
            arrays.append(values(owner, owner))
            receiver = True
        elif ":" in param and not param.startswith(("self", "&")):
            arrays.append(values(param.split(":", 1)[1].strip(), owner))
            args.append(f"v{i}")
        else:
            return None
        if arrays[-1] is None:
            return None
    call = f"v0.{name}({', '.join(args)})" if receiver else f"{owner}::{name}({', '.join(args)})"
    loops = "".join(f"for v{i} in {array} {{ " for i, array in enumerate(arrays))
    return (f'    {{ let mut hit = false; {loops}hit |= catch_unwind(AssertUnwindSafe(|| {{ bb({call}); }})).is_err(); '
            f'{"} " * len(arrays)}report("{owner}::{name}", hit); }}')


PRELUDE = """#![allow(warnings)]
use std::hint::black_box as bb;
use std::panic::{catch_unwind, set_hook, AssertUnwindSafe};
use std::sync::Mutex;
static LAST: Mutex<String> = Mutex::new(String::new());
fn report(name: &str, panicked: bool) {
    println!("{name}\\t{}\\t{}", if panicked { "panic" } else { "ok" }, LAST.lock().unwrap().replace(['\\n', '\\t'], " "));
    LAST.lock().unwrap().clear();
}
fn main() {
    set_hook(Box::new(|info| {
        let p = info.payload();
        let m = p.downcast_ref::<&str>().map(|s| s.to_string()).or_else(|| p.downcast_ref::<String>().cloned());
        *LAST.lock().unwrap() = m.unwrap_or_default();
    }));
"""


def main() -> None:
    methods = [it for it in std_panics.all_items()
               if it.kind == "method" and it.owner in OWNERS and not it.unstable and not it.unsafe]
    blocks, skipped = [], []
    for it in methods:
        block = call_block(it.owner, it.name, it.signature)
        (blocks if block else skipped).append(block or it.path)

    crate = ROOT / "target" / "brute-force"
    (crate / "src").mkdir(parents=True, exist_ok=True)
    (crate / "Cargo.toml").write_text('[package]\nname = "brute-force"\nversion = "0.0.0"\nedition = "2024"\n\n[workspace]\n')
    (crate / "src" / "main.rs").write_text(PRELUDE + "\n".join(blocks) + "\n}\n", encoding="utf-8")
    result = subprocess.run(["cargo", "run", "--quiet"], cwd=crate, capture_output=True, text=True, env=os.environ)
    if result.returncode != 0:
        sys.exit(f"the generated program failed:\n{result.stderr[-3000:]}")
    panicking = {}
    for line in result.stdout.splitlines():
        name, outcome, message = line.split("\t")
        if outcome == "panic":
            panicking[name] = message

    with open(ROOT / "clippy.toml", "rb") as f:
        config = tomllib.load(f)
    banned = {e["path"] for e in config["disallowed-methods"]}
    with open(ROOT / "tools" / "accounting.toml", "rb") as f:
        covered = [pattern(i) for rule in tomllib.load(f).get("lint", []) for i in rule["items"]]

    unaccounted = {n: m for n, m in panicking.items() if n not in banned and not any(rx.match(n) for rx in covered)}
    print(f"rustc {std_panics.rust_version()}: called {len(blocks)} methods of the integer and float types, char and bool")
    print(f"  ({len(skipped)} skipped: they take a parameter this script has no boundary values for)")
    print(f"  {len(panicking)} can panic; {len(panicking) - len(unaccounted)} of those are banned or covered by an operator lint")
    if unaccounted:
        print(f"\n{len(unaccounted)} panicking method(s) not accounted for:")
        for name, message in sorted(unaccounted.items()):
            print(f'  - {name}: {message}\n        {{ path = "{name}", reason = "..." }},')
        sys.exit(1)
    print("every panicking method is accounted for")


if __name__ == "__main__":
    main()
