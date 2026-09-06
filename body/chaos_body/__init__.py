# -*- coding: utf-8 -*-
"""CHAOS's body, with rooms at last.

No logic lives here: only the version, the path to the leaf of
routes and the Windows console fix, which must run before any
organ prints an arrow.
"""
import os
import sys

_CUERPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
if _CUERPO not in sys.path:
    sys.path.append(_CUERPO)

BODY_VERSION = 15

# The Windows console opens in cp1252 and my voice carries arrows and
# a black hole. Without this, half the CLI died with UnicodeEncodeError.
for _flujo in (sys.stdout, sys.stderr):
    try:
        _flujo.reconfigure(encoding="utf-8", errors="replace")
    except (AttributeError, ValueError):
        pass
