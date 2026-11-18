#!/usr/bin/env python3
"""Inventory of every documented panic in the installed standard library.

Reads the rustdoc HTML that ships with the toolchain (`rustup component add rust-docs`)
and lists each public item of `core`, `alloc` and `std` whose documentation has a
"Panics" section, with the path clippy uses to name it.

    tools/std_panics.py              # stable items with a "Panics" section, as TSV
    tools/std_panics.py --mentions   # add docs that mention panics or overflow without such a section
    tools/std_panics.py --all        # include unstable items

`tools/audit.py` imports this module to check that every item is accounted for.
"""
from __future__ import annotations

import argparse
import html
import os
import re
import subprocess
import sys
from dataclasses import dataclass

CRATES = ("core", "alloc", "std")
PAGE_KINDS = ("struct", "enum", "union", "trait", "fn", "macro", "primitive", "type")
# rustdoc documents both raw pointer types on one page; clippy names them separately.
POINTER_OWNERS = {"*const": "const_ptr", "*mut": "mut_ptr"}

TAG = re.compile(r"<[^>]+>")
SECTION = re.compile(r'<section id="([^"]+)"')
HEADING = re.compile(r'<h([2-6]) id="[^"]*">(?:<a class="doc-anchor"[^>]*>§</a>)?(.*?)</h[2-6]>', re.S)
SRC = re.compile(r'<a class="src[^"]*" href="[^"]*?/src/(([a-z_]+)/[^"]+)"')
CODE_HEADER = re.compile(r'<h[34] class="code-header">(.*?)</h[34]>', re.S)
EXAMPLE = re.compile(r'<div class="example-wrap.*?</pre>.*?</div>', re.S)
MENTION = re.compile(r"\bpanic|\boverflow", re.I)
PRIMITIVE = re.compile(r"\b(?:[iu](?:8|16|32|64|128|size)|f(?:16|32|64|128)|str|char|bool)\b")
DOC_LINK = re.compile(r'href="[^"#]*?(?:struct|enum|union|trait|primitive)\.(\w+)\.html#(?:ty)?method\.(\w+)"')


def text_of(fragment: str) -> str:
    """Visible text of an HTML fragment, whitespace collapsed."""
    return " ".join(html.unescape(TAG.sub("", fragment)).split())


@dataclass
class Item:
    kind: str  # macro | fn | method | trait-method | impl-method | impl
    path: str  # clippy path, or the impl header for kind == "impl"
    crate: str  # crate whose docs the path comes from
    signature: str
    unstable: bool
    deprecated: bool
    unsafe: bool
    panics: str  # text of the "Panics" section, "" if there is none
    mention: str  # first sentence that mentions panicking or overflow, when there is no section
    context: str  # impl header for methods
    src: str  # source file and line range of the definition
    links: frozenset = frozenset()  # (type, method) pairs the documentation links to
    alias: bool = False  # documented on a type-alias page

    @property
    def name(self) -> str:
        return self.path.rsplit("::", 1)[-1]

    @property
    def ident(self) -> str:
        """How the item is referred to in the accounting: its path, or its impl for trait implementations."""
        if self.kind == "impl":
            return impl_header(self.context)
        if self.kind == "impl-method":
            return f"{impl_header(self.context)}::{self.name}"
        return self.path

    @property
    def owner(self) -> str:
        return self.path.rsplit("::", 1)[0] if "::" in self.path else ""


def impl_header(header: str) -> str:
    """`impl<T: Clone> Clone for core::cell::RefCell<T> where ...` -> `impl Clone for RefCell<T>`.

    rustdoc renders the same impl with or without a where-clause and with different paths,
    depending on the page, so the generics, the where-clause and the module paths are dropped.
    """
    if header.startswith("impl<"):
        depth = 0
        for i, ch in enumerate(header):
            depth += (ch == "<") - (ch == ">" and header[i - 1] != "-")
            if depth == 0 and i > 4:
                header = "impl" + header[i + 1:]
                break
    header = re.split(r"\s*\bwhere\b", header)[0]
    return re.sub(r"\b(?:\w+::)+", "", header).strip()


def doc_root() -> str:
    sysroot = subprocess.run(["rustc", "--print", "sysroot"], check=True, capture_output=True, text=True).stdout.strip()
    root = os.path.join(sysroot, "share", "doc", "rust", "html")
    if not os.path.isdir(os.path.join(root, "std")):
        sys.exit(f"std docs not found under {root}; run `rustup component add rust-docs`")
    return root


def rust_version() -> str:
    out = subprocess.run(["rustc", "--version"], check=True, capture_output=True, text=True).stdout
    return out.split()[1]


def panics_section(doc: str) -> str:
    """Text under a heading whose title contains 'panic', up to the next heading of the same or a higher level."""
    out = []
    for m in HEADING.finditer(doc):
        if "panic" not in text_of(m.group(2)).lower():
            continue
        rest = doc[m.end():]
        end = len(rest)
        for nxt in HEADING.finditer(rest):
            if int(nxt.group(1)) <= int(m.group(1)):
                end = nxt.start()
                break
        out.append(text_of(EXAMPLE.sub(" ", rest[:end])))
    return " ".join(out).strip()


def panic_mention(doc: str) -> str:
    """First sentence of the prose (examples stripped) that mentions panicking or overflow."""
    for sentence in re.split(r"(?<=[.!?])\s+", text_of(EXAMPLE.sub(" ", doc))):
        if MENTION.search(sentence):
            return sentence
    return ""


def chunks(region: str):
    """Split a region at every `<section id=...>`; yield (id, chunk)."""
    marks = [(m.start(), m.group(1)) for m in SECTION.finditer(region)]
    for i, (start, sid) in enumerate(marks):
        yield sid, region[start: marks[i + 1][0] if i + 1 < len(marks) else len(region)]


def region(page: str, start_id: str, stop_ids: tuple[str, ...]) -> str:
    i = page.find(f'id="{start_id}"')
    if i < 0:
        return ""
    ends = [j for j in (page.find(f'<h2 id="{s}', i) for s in stop_ids) if j > 0]
    return page[i: min(ends) if ends else len(page)]


def make_item(kind, path, chunk, crate, context, page_unstable, alias=False, signature=None) -> Item:
    doc_start = chunk.find('<div class="docblock">')
    doc = chunk[doc_start:] if doc_start >= 0 else ""
    head = chunk[: doc_start if doc_start >= 0 else len(chunk)]
    if signature is None:
        header = CODE_HEADER.search(chunk)
        signature = text_of(header.group(1)) if header else ""
    panics = panics_section(doc)
    src = SRC.search(chunk)
    return Item(
        kind=kind,
        path=path,
        crate=crate,
        signature=signature,
        unstable=page_unstable or 'class="stab unstable"' in head,
        deprecated='class="stab deprecated"' in head,
        unsafe=bool(re.search(r"\bunsafe\b", signature)),
        panics=panics,
        mention="" if panics else panic_mention(doc),
        context=context,
        src=re.sub(r"(#\d+)-\d+$", r"\1", src.group(1)) if src else "",  # file#first-line identifies a definition
        links=frozenset(DOC_LINK.findall(doc)),
        alias=alias,
    )


def impl_items(owner, list_region, kind_for_methods, crate, page_unstable, alias, pointer=False):
    """Items of one implementations list: methods, plus documentation attached to the impl blocks themselves."""
    context = ""
    method_owner = owner
    for sid, chunk in chunks(list_region):
        if sid.startswith("impl-"):
            header = CODE_HEADER.search(chunk)
            context = text_of(header.group(1)) if header else ""
            if pointer:  # `impl<T> *const T` / `impl<T> *mut T`
                method_owner = next((v for k, v in POINTER_OWNERS.items() if k in context), owner)
            if kind_for_methods != "method":  # documentation on a trait impl block
                yield make_item("impl", context, chunk, crate, context, page_unstable, alias, signature=context)
        elif sid.startswith(("method.", "tymethod.")):
            name = re.sub(r"-\d+$", "", sid.split(".", 1)[1])
            yield make_item(kind_for_methods, f"{method_owner}::{name}", chunk, crate, context, page_unstable, alias)


def parse_page(root: str, rel: str) -> list[Item]:
    crate = rel.split(os.sep)[0]
    kind, name = os.path.basename(rel)[: -len(".html")].split(".", 1)
    with open(os.path.join(root, rel), encoding="utf-8") as f:
        page = f.read()
    main = page[page.find('<section id="main-content"'):]
    first_h2 = main.find("<h2 id=")
    top = main[: first_h2 if first_h2 > 0 else len(main)]
    page_unstable = 'class="stab unstable"' in top
    modules = rel.split(os.sep)[:-1]

    if kind == "primitive":
        if crate != "std":  # std's page is the superset of core's and alloc's
            return []
        owner = name
    else:
        owner = "::".join(modules + [name])

    if kind in ("fn", "macro"):  # the whole page documents one item
        decl = re.search(r'<pre class="rust item-decl"><code>(.*?)</code></pre>', top, re.S)
        return [make_item(kind, owner, main, crate, "", page_unstable, signature=text_of(decl.group(1)) if decl else "")]

    if kind == "trait":
        body = main
        for stop in ('<h2 id="implementors"', '<h2 id="foreign-impls"', '<h2 id="dyn-compatibility"'):
            j = body.find(stop)
            if j > 0:
                body = body[:j]
        items = [make_item("trait-method", f"{owner}::{re.sub(r'-\d+$', '', sid.split('.', 1)[1])}", chunk, crate,
                           f"trait {name}", page_unstable)
                 for sid, chunk in chunks(body) if sid.startswith(("method.", "tymethod."))]
        # blanket and foreign impls that carry their own documentation
        for list_id in ("implementors-list", "foreign-impls"):
            rest = main[main.find(f'id="{list_id}"'):] if f'id="{list_id}"' in main else ""
            items += [it for it in impl_items(owner, rest, "impl-method", crate, page_unstable, False) if it.kind == "impl"]
        return items

    stops = ("deref-methods", "trait-implementations", "synthetic-implementations", "blanket-implementations")
    alias = kind == "type"
    items = list(impl_items(owner, region(main, "implementations-list", stops), "method", crate, page_unstable, alias,
                            pointer=(kind == "primitive" and name == "pointer")))
    items += impl_items(owner, region(main, "trait-implementations-list", stops[2:]), "impl-method", crate,
                        page_unstable, alias)
    return items


def all_items() -> list[Item]:
    """Every documented item of core, alloc and std, one entry per definition."""
    root = doc_root()
    found: list[Item] = []
    for crate in CRATES:
        for dirpath, _dirs, files in os.walk(os.path.join(root, crate)):
            for fname in sorted(files):
                if fname.endswith(".html") and fname.split(".", 1)[0] in PAGE_KINDS:
                    found.extend(parse_page(root, os.path.relpath(os.path.join(dirpath, fname), root)))

    # A trait impl method that only repeats the trait's documentation is not a separate item.
    trait_docs = {(it.name, it.panics or it.mention) for it in found if it.kind == "trait-method"}
    found = [it for it in found if not (it.kind == "impl-method" and (it.name, it.panics or it.mention) in trait_docs)]

    # Methods shown on a type-alias page duplicate the aliased type's, unless that type is itself unstable
    # (`AtomicU32` is a stable alias of the unstable `Atomic<u32>`).
    stable_defs = {(it.name, it.src) for it in found if not it.alias and not it.unstable}
    found = [it for it in found if not (it.alias and (it.name, it.src) in stable_defs)]

    # One definition can be documented under several paths (`std::sync::Mutex`, `std::sync::poison::Mutex`,
    # `alloc::vec::Vec`, `std::vec::Vec`). Keep the path that is stable, lowest in the crate stack, and shortest.
    best: dict[tuple, Item] = {}
    for it in found:
        type_name = it.owner.rsplit("::", 1)[-1]
        if it.kind in ("impl", "impl-method"):
            # The same impl is rendered with different paths and where-clauses on different pages, and is
            # listed on the page of every type it mentions; macro-generated impls share one source location.
            name = it.name if it.kind == "impl-method" else ""
            key = (it.kind, name, it.src, tuple(sorted(set(PRIMITIVE.findall(it.context)))))
        elif it.kind in ("fn", "macro"):
            key = (it.kind, it.name, it.src)
        else:
            key = (it.kind, type_name, it.name, it.src)
        rank = (it.unstable, CRATES.index(it.crate), len(it.path))
        if key not in best or rank < best[key][0]:
            best[key] = (rank, it)
    return sorted((it for _, it in best.values()), key=lambda it: (it.path, it.context))


def inventory(mentions: bool = False, unstable: bool = False) -> list[Item]:
    """Items that document a panic."""
    return [it for it in all_items()
            if (it.panics or (mentions and it.mention)) and (unstable or not it.unstable)]


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--mentions", action="store_true", help="include docs that mention panics without a section")
    ap.add_argument("--all", action="store_true", help="include unstable items")
    args = ap.parse_args()
    for it in inventory(args.mentions, args.all):
        flags = ",".join(f for f, on in (("unstable", it.unstable), ("deprecated", it.deprecated), ("unsafe", it.unsafe)) if on)
        print("\t".join([it.kind, it.path, flags, it.context, (it.panics or "~ " + it.mention)[:300]]))


if __name__ == "__main__":
    main()
