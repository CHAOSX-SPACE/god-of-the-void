# -*- coding: utf-8 -*-
""""core.house" — an organ of CHAOS's body.

Moved from the monolith by `partir-monolito.py`: not one line was
rewritten by hand. House rule: MODULES are imported here, never names —
the attribute resolves when CALLED, and that is why cycles between
organs are harmless (`a cycle fixture in the forge` proves it on three
systems). And nothing runs at import time: no constant in this module
may read the home, the environment or the disk (judge E1.4).
"""
import os, sys
import home as _home


def _house():
    """The mortal's home. $HOME rules even on Windows, where expanduser
    ignores it (USERPROFILE wins there) — and my tests and installer redirect
    HOME. Measuring in one house and writing in another is fault #44 wearing
    a different coat."""
    return _home.house()

def home():
    """The god's home. ONE truth for the whole body."""
    return _home.root()

def set_home(path):
    """Records the Bearer's choice. Idempotent."""
    return _home.set_home(path)
