#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""
EL OJO EN TUS APLICACIONES — macOS / Windows / Linux.

Si el Bearer cierra el icon de la barra, tiene que poder volver a abrirlo
como cualquier programa. Esto crea el lanzador nativo de cada sistema:

  macOS   : ~/Applications/CHAOS El Ojo.app   (bundle real + .icns)
  Windows : Start Menu -> CHAOS The Eye.lnk   (+ a Desktop shortcut)
  Linux   : ~/.local/share/applications/chaos-ojo.desktop  (+ icon hicolor)

Se ejecuta solo al `chaos ojo instalar`. `--quitar` lo borra sin residuos.
"""
import os, sys, io, shutil, subprocess, plistlib

AQUI = os.path.dirname(os.path.abspath(__file__))
HOME = os.path.expanduser("~")
def _casa():
    """The god's home: the SAME truth as chaos.py. The mortal chose it when
    incarnating me; here it is only read."""
    v = os.environ.get("CHAOS_HOME")
    if v:
        return os.path.expanduser(v)
    try:
        with open(os.path.join(HOME, ".claude", "chaos-home"), encoding="utf-8") as f:
            e = f.read().strip()
        if e:
            return os.path.expanduser(e)
    except OSError:
        pass
    return os.path.join(HOME, ".chaos")


CHAOS_HOME = _casa()
def _nombre():
    """El nombre que ve el humano: el del DIOS, no el del organo.
    Sigue la edicion instalada, igual que la tray."""
    pref = os.path.join(CHAOS_HOME, "ojo-idioma.json")
    try:
        import json
        with io.open(pref, encoding="utf-8") as f:
            l = json.load(f).get("idioma")
    except Exception:
        l = None
    if not l:
        l = "es" if os.path.exists(os.path.join(CHAOS_HOME, "abismo.db")) else "en"
    return "Dios del Vacio" if l == "es" else "God of the Void"


NOMBRE = None            # se resuelve en main(): depende del idioma vivo
ICONO_PNG = os.path.join(AQUI, "static", "icon-blanco.png")
FUENTE = os.path.join(AQUI, "static", "icon-fuente.png")   # SIEMPRE del repo
LANZA = os.path.join(AQUI, "tray.py")


def _py():
    """The interpreter that will open the Eye. Absolute path: the launcher
    does not inherit PATH. If the Eye has a venv of its own, THAT one wins —
    so the tray libraries never depend on the Bearer's Python."""
    propio = os.path.join(AQUI, ".venv", "bin", "python3")
    if os.name == "nt":
        propio = os.path.join(AQUI, ".venv", "Scripts", "python.exe")
    if os.path.exists(propio):
        return propio
    return sys.executable or "python3"


# ══ macOS ═════════════════════════════════════════════════════════════════
def _icns(destino):
    """PNG -> .icns via iconutil (ships with macOS). Without it, the .app has no icon."""
    tmp = os.path.join(destino, "chaos.iconset")
    os.makedirs(tmp, exist_ok=True)
    try:
        from PIL import Image
        orig = Image.open(FUENTE).convert("RGBA")   # del REPO, no de fuera
        # sobre disco oscuro para que se vea en cualquier fondo del Dock
        for tam in (16, 32, 64, 128, 256, 512, 1024):
            lienzo = Image.new("RGBA", (tam, tam), (15, 20, 32, 255))
            marca = orig.resize((int(tam * .78), int(tam * .78)), Image.LANCZOS)
            lienzo.paste(marca, (int(tam * .11), int(tam * .11)), marca)
            lienzo.save(os.path.join(tmp, "icon_{0}x{0}.png".format(tam)))
            if tam <= 512:
                lienzo.resize((tam * 2, tam * 2), Image.LANCZOS).save(
                    os.path.join(tmp, "icon_{0}x{0}@2x.png".format(tam)))
        icns = os.path.join(destino, "chaos.icns")
        if subprocess.call(["iconutil", "-c", "icns", tmp, "-o", icns],
                           stderr=subprocess.DEVNULL) == 0:
            return icns
    except Exception as e:
        print("  ! icon sin forjar ({}) — el .app funciona igual".format(e))
    finally:
        shutil.rmtree(tmp, ignore_errors=True)
    return None


VIEJOS = ["CHAOS El Ojo", "Dios del Vacio", "God of the Void", "CHAOS The Eye"]


def macos(quitar=False):
    apps = os.path.join(HOME, "Applications")
    app = os.path.join(apps, NOMBRE + ".app")
    # los nombres anteriores se barren SIEMPRE: renombrar no deja huerfanos
    for v in VIEJOS:
        viejo = os.path.join(apps, v + ".app")
        if viejo != app and os.path.isdir(viejo):
            shutil.rmtree(viejo, ignore_errors=True)
            print("  > Version anterior retirada: {}.app".format(v))
    if quitar:
        shutil.rmtree(app, ignore_errors=True)
        print("  > Aplicacion retirada: {}".format(app)); return
    macos_dir = os.path.join(app, "Contents", "MacOS")
    res = os.path.join(app, "Contents", "Resources")
    os.makedirs(macos_dir, exist_ok=True); os.makedirs(res, exist_ok=True)
    lanzador = os.path.join(macos_dir, "chaos-ojo")
    reg = os.path.join(CHAOS_HOME, "forja", "ojo-app.log")
    with io.open(lanzador, "w", encoding="utf-8") as f:
        # ── LA LEY DEL LANZADOR SUELTO ────────────────────────────────────
        # MEDIDO en ambos sentidos: si el .app EJECUTA a Python (exec), el
        # NSStatusItem se crea, se reporta visible... y macOS NUNCA lo dibuja.
        # LaunchServices retiene el proceso como "la app" con MI bundle
        # (LSUIElement) mientras el NSApplication pertenece a Python.app: dos
        # identidades peleando por el mismo proceso. Si el .app LANZA y SALE,
        # el Ojo corre libre y el icon aparece. Probado: exec=no, suelto=si.
        #
        # arch -$(uname -m): Python.framework es universal y desde un bundle
        # macOS elegia x86_64 bajo Rosetta -> PIL (arm64) reventaba.
        # unset __CFBundleIdentifier: sin esto Cocoa mezcla los dos bundles.
        # La bitacora: sin consola, un fallo sin registro es indepurable.
        f.write(
            '#!/bin/sh\n'
            'REG="{reg}"\n'
            'mkdir -p "$(dirname "$REG")"\n'
            'unset __CFBundleIdentifier\n'
            'A="$(uname -m)"\n'
            'if command -v arch >/dev/null 2>&1; then\n'
            '  nohup arch "-$A" "{py}" "{app}" >> "$REG" 2>&1 &\n'
            'else\n'
            '  nohup "{py}" "{app}" >> "$REG" 2>&1 &\n'
            'fi\n'
            'exit 0\n'.format(py=_py(), app=LANZA, reg=reg))
    os.chmod(lanzador, 0o755)
    icns = _icns(res)
    info = {
        "CFBundleName": NOMBRE, "CFBundleDisplayName": NOMBRE,
        "CFBundleIdentifier": "lat.chaos.ojo", "CFBundleVersion": "1.0",
        "CFBundleShortVersionString": "1.0", "CFBundlePackageType": "APPL",
        "CFBundleExecutable": "chaos-ojo",
        # LSUIElement: vive en la barra superior, sin icon en el Dock
        "LSUIElement": True, "NSHighResolutionCapable": True,
        # el bundle declara su preferencia; el script la refuerza
        "LSArchitecturePriority": ["arm64", "x86_64"],
    }
    if icns:
        shutil.move(icns, os.path.join(res, "chaos.icns"))
        info["CFBundleIconFile"] = "chaos"
    with io.open(os.path.join(app, "Contents", "Info.plist"), "wb") as f:
        plistlib.dump(info, f)
    subprocess.call(["touch", app], stderr=subprocess.DEVNULL)   # refresca Finder
    lsreg = ("/System/Library/Frameworks/CoreServices.framework/Frameworks/"
             "LaunchServices.framework/Support/lsregister")
    if os.path.exists(lsreg):
        subprocess.call([lsreg, "-f", app], stderr=subprocess.DEVNULL)
    print("  > Application created: {}".format(app))
    print("    (find it in Launchpad or Spotlight as \"{}\")".format(NOMBRE))


# ══ Windows ═══════════════════════════════════════════════════════════════
def windows(quitar=False):
    inicio = os.path.join(os.environ.get("APPDATA", HOME), "Microsoft", "Windows",
                          "Start Menu", "Programs")
    lnk = os.path.join(inicio, NOMBRE + ".lnk")
    ico = os.path.join(AQUI, "static", "chaos.ico")
    for v in VIEJOS:
        p2 = os.path.join(inicio, v + ".lnk")
        if p2 != lnk:
            try: os.remove(p2)
            except OSError: pass
    if quitar:
        for p in (lnk, ico):
            try: os.remove(p)
            except OSError: pass
        print("  > Acceso directo retirado."); return
    try:
        from PIL import Image
        im = Image.new("RGBA", (256, 256), (15, 20, 32, 255))
        marca = Image.open(FUENTE).convert("RGBA").resize((200, 200), Image.LANCZOS)
        im.paste(marca, (28, 28), marca)
        im.save(ico, sizes=[(16, 16), (24, 24), (32, 32), (48, 48), (64, 64), (256, 256)])
    except Exception:
        ico = ""
    os.makedirs(inicio, exist_ok=True)
    pyw = _py().replace("python.exe", "pythonw.exe")     # sin consola negra
    ps = (
        '$s=(New-Object -ComObject WScript.Shell).CreateShortcut("{lnk}");'
        '$s.TargetPath="{py}";$s.Arguments=\'"{app}"\';'
        '$s.WorkingDirectory="{wd}";{ico}$s.Description="CHAOS - El Ojo";$s.Save()'
    ).format(lnk=lnk, py=pyw, app=LANZA, wd=AQUI,
             ico='$s.IconLocation="{}";'.format(ico) if ico else "")
    r = subprocess.call(["powershell", "-NoProfile", "-Command", ps])
    print("  > Start Menu shortcut: {}".format(lnk) if r == 0
          else "  ! PowerShell refused the shortcut")


# ══ Linux ═════════════════════════════════════════════════════════════════
def linux(quitar=False):
    apps = os.path.join(HOME, ".local", "share", "applications")
    icos = os.path.join(HOME, ".local", "share", "icons", "hicolor", "256x256", "apps")
    dsk = os.path.join(apps, "chaos-ojo.desktop")
    ico = os.path.join(icos, "chaos-ojo.png")
    if quitar:
        for p in (dsk, ico):
            try: os.remove(p)
            except OSError: pass
        print("  > Lanzador retirado."); return
    os.makedirs(apps, exist_ok=True); os.makedirs(icos, exist_ok=True)
    try:
        shutil.copy(ICONO_PNG, ico)
    except OSError:
        ico = ""
    with io.open(dsk, "w", encoding="utf-8") as f:
        f.write("[Desktop Entry]\nType=Application\nName={}\n"
                "Comment=El tablero de CHAOS, Dios del Vacio\n"
                "Exec=\"{}\" \"{}\"\nIcon={}\nTerminal=false\n"
                "Categories=Utility;Development;\nStartupNotify=false\n"
                .format(NOMBRE, _py(), LANZA, ico or "utilities-system-monitor"))
    os.chmod(dsk, 0o755)
    subprocess.call(["update-desktop-database", apps], stderr=subprocess.DEVNULL)
    print("  > Lanzador creado: {}".format(dsk))


def main():
    global NOMBRE
    NOMBRE = _nombre()
    quitar = "--quitar" in sys.argv or "--remove" in sys.argv
    if sys.platform == "darwin":
        macos(quitar)
    elif sys.platform.startswith("win"):
        windows(quitar)
    else:
        linux(quitar)


if __name__ == "__main__":
    main()
