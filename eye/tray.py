#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""
LA BANDEJA DEL OJO — el icono junto al reloj · macOS / Windows / Linux.

macOS law (known fault, never repeat it): pystray DEMANDS the main
principal. La tray lo toma; el servidor HTTP corre en daemon thread.
Si pystray falta o muere (16 meses sin commit — riesgo declarado), el Ojo
degrades: server.py straight in the browser. The tray is a shortcut, not the only road.
"""
import os, sys, json, threading, subprocess, webbrowser, io

AQUI = os.path.dirname(os.path.abspath(__file__))
def _casa():
    """La casa del dios: la MISMA verdad que chaos.py, sin importarlo (los
    hooks must be instant). Env > the Bearer's choice > default."""
    v = os.environ.get("CHAOS_HOME")
    if v:
        return os.path.expanduser(v)
    try:
        with open(os.path.join(os.path.expanduser("~"), ".claude", "chaos-home"),
                  encoding="utf-8") as f:
            e = f.read().strip()
        if e:
            return os.path.expanduser(e)
    except OSError:
        pass
    return os.path.join(os.path.expanduser("~"), ".chaos")


CHAOS_HOME = _casa()
PREF = os.path.join(CHAOS_HOME, "ojo-idioma.json")

NOMBRES = {"es": "Dios del Vacio", "en": "God of the Void"}

TXT = {
    "es": {"abrir": "Abrir", "cerrar": "Cerrar", "idioma": "Language: English"},
    "en": {"abrir": "Open", "cerrar": "Close", "idioma": "Language: Espanol"},
}


def _idioma():
    try:
        with io.open(PREF, encoding="utf-8") as f:
            return json.load(f).get("idioma", "es")
    except Exception:
        # edition: if only the English DB exists, start in English
        return "es" if os.path.exists(os.path.join(CHAOS_HOME, "abismo.db")) else "en"


def _guardar_idioma(l):
    try:
        os.makedirs(CHAOS_HOME, exist_ok=True)
        with io.open(PREF, "w", encoding="utf-8") as f:
            json.dump({"idioma": l}, f)
    except Exception:
        pass


def _icono_para_barra():
    """macOS: NEGRO con alfa + template image -> el sistema lo invierte solo
    segun la barra (clara u oscura). Falla real: el icono blanco era INVISIBLE
    en barra clara. Win/Linux: blanco (sus trays son oscuras)."""
    from PIL import Image
    negro = os.path.join(AQUI, "static", "icono-negro.png")
    blanco = os.path.join(AQUI, "static", "icono-blanco.png")
    if sys.platform == "darwin":
        im = Image.open(negro if os.path.exists(negro) else blanco).convert("RGBA")
        return im.resize((20, 20), Image.LANCZOS)
    im = Image.open(blanco).convert("RGBA")
    return im.resize((32, 32), Image.LANCZOS)


def _politica_macos():
    """ACCESSORY *antes* de arrancar el bucle: si no, NSApplication se vuelve
    app normal y aparece el icono generico de Python en el Dock.
    Debe correr en el HILO PRINCIPAL — AppKit no admite otra cosa."""
    if sys.platform != "darwin":
        return
    try:
        import AppKit
        AppKit.NSApplication.sharedApplication().setActivationPolicy_(
            AppKit.NSApplicationActivationPolicyAccessory)
    except Exception as e:
        print("[OJO] no pude salir del Dock: {}".format(e), flush=True)


def _programar_template(icono):
    """TEMPLATE IMAGE — la unica forma de que el icono se vea SIEMPRE.

    Falla real y visible: un icono de color fijo desaparece. El negro se
    tragaba la barra oscura; el blanco, la clara. Con template, macOS lo tinta
    solo segun el fondo real (que ademas depende del fondo de pantalla).

    La imagen NO existe hasta que pystray dibuja el item, y setTemplate_ debe
    correr en el HILO PRINCIPAL: por eso va en un NSTimer del bucle de Cocoa,
    no en el callback `setup` (que pystray corre en OTRO hilo).
    """
    if sys.platform != "darwin":
        return
    try:
        import AppKit

        def poner(_timer):
            try:
                boton = icono._status_item.button()
                img = boton.image()
                if img is not None:
                    img.setTemplate_(True)
                    boton.setImage_(img)
            except Exception as e:
                print("[OJO] template fallido: {}".format(e), flush=True)

        AppKit.NSTimer.scheduledTimerWithTimeInterval_repeats_block_(0.5, False, poner)
    except Exception as e:
        print("[OJO] sin template: {}".format(e), flush=True)


def main():
    # 1. El servidor, en daemon thread — la tray necesita el principal (macOS).
    sys.path.insert(0, AQUI)
    import server
    srv = server.Servidor(("127.0.0.1", 0), server.Ojo)
    puerto = srv.server_address[1]
    url = "http://127.0.0.1:{}/?t={}".format(puerto, server.TOKEN)
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    print("[OJO] {}".format(url), flush=True)

    # 2. La tray. Si pystray no puede, el browser basta.
    try:
        import pystray
    except ImportError:
        print("[OJO] sin pystray: degrado al browser.", flush=True)
        webbrowser.open(url)
        threading.Event().wait()                       # el server sigue vivo
        return

    idioma = _idioma()

    def abrir(icon, item):
        webbrowser.open(url)

    def cerrar(icon, item):
        icon.stop()                                    # muere la tray → muere todo

    def cambiar_idioma(icon, item):
        nonlocal idioma
        idioma = "en" if idioma == "es" else "es"
        _guardar_idioma(idioma)
        icon.menu = _menu()
        icon.update_menu()
        icon.title = "{} - CHAOS".format(NOMBRES.get(idioma, NOMBRES["es"]))
        # el nombre del dios cambia tambien en Aplicaciones: si no, quedaria
        # un lanzador en un idioma y una tray en otro
        app = os.path.join(AQUI, "install-app.py")
        if os.path.exists(app):
            subprocess.Popen([sys.executable, app],
                             stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)

    def _menu():
        t = TXT[idioma]
        return pystray.Menu(
            pystray.MenuItem(t["abrir"], abrir, default=True),
            pystray.MenuItem(t["cerrar"], cerrar),
            pystray.Menu.SEPARATOR,
            pystray.MenuItem(t["idioma"], cambiar_idioma),
        )

    nombre = NOMBRES.get(idioma, NOMBRES["es"])
    _politica_macos()                       # ANTES de crear nada: fuera del Dock
    icono = pystray.Icon("chaos-ojo", _icono_para_barra(),
                         "{} - CHAOS".format(nombre), _menu())
    _programar_template(icono)              # se aplica solo, ya en el bucle
    icono.run()                             # HILO PRINCIPAL - ley de macOS


if __name__ == "__main__":
    main()
