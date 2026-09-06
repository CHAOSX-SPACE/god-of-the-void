#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""
LA THE SINGLE GUARD — what a body would lose if another overwrites it.

Born of PLAN-ADN v2 §2.0: the guard logic was about to be copied into four
places (manual F0, `sow`, `install.py`, the forge) — four hand copies of the
guard AGAINST hand copies, the disease dressed as the cure. It lives here
ONCE; everyone calls it.

NEVER measure with `diff | grep -c`: a diff counts a replacement as a loss
(fault #236). What is compared is what would TRULY be lost — functions,
dispatcher commands and tables that exist only in the destination.

CLI (for shell, e.g. the forge):
    python3 dna-guard.py <source> <destination>
    → exits 0 if copying source→destination loses nothing; 1 naming losses.

API (for install.py and `chaos sow`):
    would_lose(source, destination) → dict of losses; empty = safe.
"""
import ast, io, os, re, sys


def _whole_body(path):
    """The body is a TREE since the split: the gate plus its package. A guard
    that reads only `chaos.py` sees a 51-line door and declares that 130
    functions died — it refused a healthy forge the first time it met one."""
    parts = [io.open(path, encoding="utf-8").read()]
    base = os.path.dirname(os.path.abspath(path))
    for pkg in ("chaos_body", "chaos_cuerpo"):
        for root, _, files in os.walk(os.path.join(base, pkg)):
            if "__pycache__" in root:
                continue
            for f in sorted(files):
                if f.endswith(".py"):
                    parts.append(io.open(os.path.join(root, f),
                                         encoding="utf-8").read())
    return "\n".join(parts)


def functions(path):
    try:
        tree = ast.parse(_whole_body(path))
    except SyntaxError:
        return {"<BROKEN SYNTAX>"}       # an unreadable side is never declared empty
    return {n.name for n in ast.walk(tree) if isinstance(n, ast.FunctionDef)}


def commands(path):
    return set(re.findall(r'elif cmd == "([\w-]+)"', _whole_body(path)))


def tables(path):
    return set(re.findall(r'CREATE (?:VIRTUAL )?TABLE (?:IF NOT EXISTS )?(\w+)',
                          _whole_body(path)))


def would_lose(source, destination):
    """What the DESTINATION has that the SOURCE lacks — i.e. what dies if
    the source overwrites it. Empty = the copy is sowing, not amputation."""
    out = {}
    for name, pull in (("functions", functions), ("commands", commands),
                       ("tables", tables)):
        only_dest = sorted(pull(destination) - pull(source))
        if only_dest:
            out[name] = only_dest
    return out


def main(argv):
    if len(argv) != 3:
        print("usage: dna-guard.py <source> <destination>"); return 2
    p = would_lose(argv[1], argv[2])
    if not p:
        print("GUARD: safe — the destination loses nothing"); return 0
    print("GUARD: COPYING WOULD AMPUTATE the destination. It would lose:")
    for k, v in p.items():
        print("  %s: %s" % (k, ", ".join(v)))
    return 1


if __name__ == "__main__":
    sys.exit(main(sys.argv))
