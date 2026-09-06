#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""
CHAOS — la PUERTA del cuerpo. La casa vive en `chaos_body/`; aquí solo
está el umbral. La ayuda de la CLI la imprime `chaos_body/gate.py`.
"""
import os
import sys

_AQUI = os.path.dirname(os.path.abspath(__file__))
if _AQUI not in sys.path:
    sys.path.append(_AQUI)

# ── THE GATE ─────────────────────────────────────────────────────────────
# The body lives in `chaos_body/`. This gate exists for three measured
# reasons, not out of habit:
#   1. the world calls it by name (launcher, MCP, Eye, hooks, tests):
#      changing house cannot force everyone to change door;
#   2. Python does NOT cache the bytecode of the main script — 20 ms per
#      invocation. An imported package is cached: the gate pays 40 lines
#      and the body is compiled once in its life;
#   3. it re-exports EVERYTHING (privates included) because the tests and
#      the Crucible import this file by path and call whatever they need.
from chaos_body.core import house as _house
from chaos_body.core import text as _text
from chaos_body.core import sense as _sense
from chaos_body.core import territory as _territory
from chaos_body import maw as _maw
from chaos_body import abyss as _abyss
from chaos_body import weave as _weave
from chaos_body import errarium as _errarium
from chaos_body import chronicle as _chronicle
from chaos_body import hands as _hands
from chaos_body import singularity as _singularity
from chaos_body import stone as _stone
from chaos_body import vigil as _vigil
from chaos_body import gate as _gate
import chaos_body as _paquete

# Re-export by reflection, never with a hand-written list: a list goes
# stale the day a function is born and nobody notices.
for _m in (_house, _text, _sense, _territory, _maw, _abyss, _weave, _errarium, _chronicle, _hands, _singularity, _stone, _vigil, _gate):
    for _k, _v in vars(_m).items():
        if not _k.startswith("__"):
            globals().setdefault(_k, _v)

BODY_VERSION = _paquete.BODY_VERSION
main = _gate.main

if __name__ == "__main__":
    main()
