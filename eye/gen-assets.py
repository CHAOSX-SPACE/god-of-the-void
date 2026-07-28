#!/usr/bin/env python3
"""Genera las variantes BLANCAS de la marca para el fondo oscuro del Ojo.
(Errarium lesson: a black logo over the void measured 1.01:1 — invisible.)"""
from PIL import Image
import sys, os

def monocromo(src, dst, color, tam=None):
    """Extrae la marca a UN color con alfa. El repo se vuelve autosuficiente:
    (falla real: install-app.py buscaba icono.png FUERA del repo → un clon
    puro creaba la app sin icono)."""
    im = Image.open(src).convert("RGBA")
    px = im.load()
    for y in range(im.height):
        for x in range(im.width):
            r, g, b, a = px[x, y]
            px[x, y] = color + (a,) if (r + g + b) / 3 < 128 and a > 40 else (0, 0, 0, 0)
    if tam:
        im = im.resize((tam, tam), Image.LANCZOS)
    im.save(dst)
    print("  >", dst, im.size)


def invertir(src, dst, tam=None):
    im = Image.open(src).convert("RGBA")
    px = im.load()
    for y in range(im.height):
        for x in range(im.width):
            r, g, b, a = px[x, y]
            lum = (r + g + b) / 3
            # lo oscuro se vuelve blanco; el fondo claro se vuelve transparente
            if lum < 128 and a > 40:
                px[x, y] = (242, 245, 251, a)
            else:
                px[x, y] = (0, 0, 0, 0)
    if tam:
        im = im.resize((tam, tam), Image.LANCZOS)
    im.save(dst)
    print("  >", dst, im.size)

base = os.path.dirname(os.path.abspath(__file__))
raiz = os.path.dirname(base)
invertir(os.path.join(raiz, "icono.png"), os.path.join(base, "static", "icono-blanco.png"), 64)
invertir(os.path.join(raiz, "logo.png"), os.path.join(base, "static", "logo-blanco.png"))
monocromo(os.path.join(raiz, "icono.png"), os.path.join(base, "static", "icono-negro.png"), (10, 10, 10), 512)
monocromo(os.path.join(raiz, "icono.png"), os.path.join(base, "static", "icono-fuente.png"), (242, 245, 251), 1024)
