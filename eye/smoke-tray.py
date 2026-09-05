#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""OJ-3 · THE TRAY, MEASURED IN THREE WORLDS — or declared.

Hunger #1 has been open since the day the tray was forged: `pystray` had only
ever run on one Mac. This does not fake a tray in CI — a tray needs a session
and a screen. It measures what CAN be measured on a headless runner:

  1. pystray imports, and which backend it chose on THIS system;
  2. the icon image builds (that is Pillow plus my own drawing);
  3. `tray.py` compiles and its macOS law is intact (main thread).

What cannot be measured without a screen is printed as a LIMIT, and the README
carries that same limit. A green here is not "the tray works": it is "nothing
in the tray is broken before the screen".

    PYSTRAY_BACKEND=xorg xvfb-run -a python3 smoke-tray.py
"""
import ast
import io
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))


def main():
    print("system: {} · PYSTRAY_BACKEND={}".format(
        sys.platform, os.environ.get("PYSTRAY_BACKEND", "(automatic)")))
    ok = True

    src = io.open(os.path.join(HERE, "tray.py"), encoding="utf-8").read()
    try:
        ast.parse(src)
        print("  OK tray.py compiles")
    except SyntaxError as e:
        print("  X tray.py does not compile: {}".format(e)); ok = False
    # El salto de línea partía «main\nthread» y mi propia sonda lo daba por
    # desaparecido: se normaliza el espacio antes de buscar (órgano 17).
    plano = " ".join(src.split())
    if "main thread" in plano or "hilo principal" in plano:
        print("  OK the macOS law is still written down (main thread)")
    else:
        print("  X the macOS law vanished from the file"); ok = False

    try:
        import pystray
        backend = getattr(pystray, "Icon", None)
        print("  OK pystray {} · backend class: {}".format(
            getattr(pystray, "__version__", "?"),
            getattr(backend, "__module__", "?")))
    except Exception as e:
        print("  .  pystray does not live here ({}): the Eye falls back to the"
              " browser, which is its declared plan B".format(type(e).__name__))
        return 0 if ok else 1

    try:
        sys.path.insert(0, HERE)
        import importlib.util
        spec = importlib.util.spec_from_file_location("tray_mod",
                                                      os.path.join(HERE, "tray.py"))
        m = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(m)
        img = m._icono_para_barra()
        print("  OK the bar icon builds: {}x{}".format(*img.size))
    except Exception as e:
        print("  X the icon did not build: {}: {}".format(type(e).__name__, e))
        ok = False

    print("  .  LIMIT: a real tray needs a session and a screen. This runner"
          " has neither, so what is measured is everything up to that point.")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
