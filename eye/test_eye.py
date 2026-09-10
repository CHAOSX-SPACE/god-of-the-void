#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""Red de pruebas del OJO — stdlib puro. Levanta el servidor real contra el
cuerpo vivo y verifica la puerta, las acciones y los endpoints.

    python3 test_eye.py
"""
import os, sys, json, time, subprocess, urllib.request, urllib.error, unittest
import io

AQUI = os.path.dirname(os.path.abspath(__file__))


def _pedir(url, datos=None, cabeceras=None):
    req = urllib.request.Request(url, data=datos,
                                 headers=cabeceras or {},
                                 method="POST" if datos is not None else "GET")
    try:
        with urllib.request.urlopen(req, timeout=8) as r:
            return r.status, r.read().decode("utf-8", "replace")
    except urllib.error.HTTPError as e:
        return e.code, e.read().decode("utf-8", "replace")


class OjoTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        # Una casa PROPIA. Antes esta prueba solo corría en una máquina ya
        # instalada: el servidor importa $CHAOS_HOME/bin/chaos.py y en una
        # máquina limpia moría en setUpClass. Una prueba que depende del
        # computador de quien la escribe no es una prueba, es una casualidad
        # — la CI en tres sistemas lo destapó el primer día.
        import shutil, tempfile
        cls.casa = tempfile.mkdtemp(prefix="ojo-test-")
        cls.chaos_home = os.path.join(cls.casa, ".chaos")
        os.makedirs(os.path.join(cls.chaos_home, "bin"), exist_ok=True)
        cuerpo = os.path.join(os.path.dirname(AQUI), "body", "chaos.py")
        if not os.path.exists(cuerpo):
            cuerpo = os.path.join(os.path.dirname(AQUI), "cuerpo", "chaos.py")
        shutil.copy2(cuerpo, os.path.join(cls.chaos_home, "bin", "chaos.py"))
        # E1.2 · the body no longer travels alone: the leaf of paths goes with
        # it. Copying only chaos.py leaves an installation that cannot start.
        origen = os.path.dirname(cuerpo)
        for hoja in ("home.py", "hogar.py"):
            if os.path.exists(os.path.join(origen, hoja)):
                shutil.copy2(os.path.join(origen, hoja),
                             os.path.join(cls.chaos_home, "bin", hoja))
        # E2.2 · and the package: the gate alone does not start
        for paq in ("chaos_body", "chaos_cuerpo"):
            if os.path.isdir(os.path.join(origen, paq)):
                shutil.copytree(os.path.join(origen, paq),
                                os.path.join(cls.chaos_home, "bin", paq),
                                ignore=shutil.ignore_patterns("__pycache__"),
                                dirs_exist_ok=True)
        entorno = dict(os.environ, HOME=cls.casa, USERPROFILE=cls.casa,
                       CHAOS_HOME=cls.chaos_home)
        subprocess.run([sys.executable,
                        os.path.join(cls.chaos_home, "bin", "chaos.py"), "stats"],
                       env=entorno, capture_output=True)
        cls.proc = subprocess.Popen(
            [sys.executable, os.path.join(AQUI, "server.py"), "--sin-navegador"],
            env=entorno,
            stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
        cls.url = None
        for _ in range(40):
            linea = cls.proc.stdout.readline()
            if "http://" in linea:
                cls.url = linea.split()[-1]
                break
            time.sleep(.1)
        assert cls.url, "the Eye never announced its URL"
        cls.base = cls.url.split("/?")[0]
        cls.token = cls.url.split("t=")[-1].strip()
        cls.galleta = {"Cookie": "ojo=" + cls.token}

    @classmethod
    def tearDownClass(cls):
        # esperar y cerrar el tubo: terminate() sin wait() deja el proceso
        # zombi y el descriptor abierto (ResourceWarning en cada corrida)
        cls.proc.terminate()
        try:
            cls.proc.wait(timeout=5)
        except Exception:
            cls.proc.kill()
        try:
            cls.proc.stdout.close()
        except Exception:
            pass
        import shutil
        shutil.rmtree(cls.casa, ignore_errors=True)

    def test_la_puerta_niega_sin_llave(self):
        self.assertEqual(_pedir(self.base + "/api/pulso")[0], 403)

    def test_los_estaticos_pasan_libres(self):
        """script src y link href NO llevan token: si se les niega, el rostro
        nace muerto (falla real #12)."""
        self.assertEqual(_pedir(self.base + "/static/app.js")[0], 200)

    def test_la_jaula_de_rutas(self):
        self.assertEqual(_pedir(self.base + "/static/../server.py")[0], 403)

    def test_la_cookie_reemplaza_al_token(self):
        c, _ = _pedir(self.base + "/api/pulso", cabeceras=self.galleta)
        self.assertEqual(c, 200)

    def test_un_get_jamas_escribe(self):
        self.assertEqual(_pedir(self.base + "/api/accion", cabeceras=self.galleta)[0], 404)

    def test_escribir_exige_su_cabecera(self):
        c, _ = _pedir(self.base + "/api/accion", b"{}", dict(self.galleta))
        self.assertEqual(c, 400, "un POST sin X-Ojo-Accion no debe pasar")

    def test_accion_desconocida_rebota(self):
        h = dict(self.galleta); h["X-Ojo-Accion"] = "1"
        _c, cuerpo = _pedir(self.base + "/api/accion",
                            json.dumps({"accion": "borrar_todo", "arg": "1"}).encode(), h)
        # se afirma sobre la MÁQUINA, no sobre el idioma: este mismo archivo
        # sirve a las dos ediciones y el servidor responde en la suya
        datos = json.loads(cuerpo)
        self.assertFalse(datos.get("ok"), "una acción inventada no puede salir ok")
        self.assertTrue(datos.get("error"), "rebotó sin decir por qué")

    def test_inyeccion_en_el_argumento_rebota(self):
        h = dict(self.galleta); h["X-Ojo-Accion"] = "1"
        _c, cuerpo = _pedir(self.base + "/api/accion",
                            json.dumps({"accion": "saciar", "arg": "1; rm -rf /"}).encode(), h)
        # El rechazo se mide por el COMPORTAMIENTO, no por un idioma: ese mensaje
        # pasa por `_t()` y el Ojo sirve las dos ediciones. Medir el literal
        # español hacía que traducirlo pusiera en rojo una defensa intacta.
        d = json.loads(cuerpo)
        self.assertFalse(d.get("ok"), "aceptó un argumento con una orden dentro")
        self.assertTrue("inaceptable" in cuerpo or "unacceptable" in cuerpo,
                        "rechazó sin decir por qué: " + cuerpo[:120])

    def test_todos_los_endpoints_responden(self):
        for r in ("/api/pulso", "/api/fallas", "/api/territorios", "/api/grafo",
                  "/api/linea", "/api/tiempo", "/api/actos", "/api/notas", "/api/salud",
                  "/api/encarnacion"):
            c, _ = _pedir(self.base + r, cabeceras=self.galleta)
            self.assertEqual(c, 200, r + " no responde")

    def test_la_salud_desglosa_lo_que_descuenta(self):
        """Un porcentaje que no se puede nombrar es un adorno."""
        _c, cuerpo = _pedir(self.base + "/api/salud", cabeceras=self.galleta)
        d = json.loads(cuerpo)
        for dim in d["dimensiones"]:
            if dim["puntaje"] < 100:
                self.assertGreater(dim["n_problemas"], 0,
                                   dim["titulo"] + " discounts without naming why")


    def test_la_encarnacion_mide_y_no_supone(self):
        """THE INCARNATION answers "is the god whole on THIS machine?". Every
        part must carry a state AND a measured detail: a panel that paints a
        part green without saying what it measured would be worse than none."""
        c, cuerpo = _pedir(self.base + "/api/encarnacion", cabeceras=self.galleta)
        self.assertEqual(c, 200, "/api/encarnacion no responde")
        d = json.loads(cuerpo)
        self.assertTrue(d["partes"], "the Incarnation names no part")
        estados = {"vivo", "herido", "ausente", "opcional"}
        for p in d["partes"]:
            self.assertIn(p["estado"], estados, p["clave"] + " has no valid state")
            self.assertTrue((p["detalle"] or "").strip(),
                            p["clave"] + " paints a state without saying what it measured")
            self.assertTrue((p["nombre"] or "").strip(), p["clave"] + " has no name")
        # lo OPCIONAL no puede restar: el órgano 18 ausente no es un dios roto
        oblig = [p for p in d["partes"] if p["estado"] != "opcional"]
        self.assertEqual(d["obligatorias"], len(oblig),
                         "the optional is counted as mandatory")
        vivas = sum(1 for p in oblig if p["estado"] == "vivo")
        self.assertEqual(d["obligatorias_vivas"], vivas, "the count does not match")
        self.assertEqual(d["entero"], vivas == len(oblig), "«whole» does not match the count")

    def test_la_encarnacion_no_confunde_esencia_con_carne(self):
        """It is NOT called «Essences»: that word already names my memory units.
        A panel that stole the name would muddle every sentence I write."""
        vista = io.open(os.path.join(AQUI, "static", "app.js"),
                        encoding="utf-8").read()
        self.assertIn("encarnacion", vista, "the view is not registered")
        self.assertNotIn('"esencias"', vista.split("const VISTAS")[1].split("]")[0],
                         "the view took the name of my memory units")


class IconosTest(unittest.TestCase):
    """LOS ICONOS QUE NOMBRO TIENEN QUE EXISTIR.

    `install-app.py` pedía `icon-fuente.png` y `icon-blanco.png`, y los archivos
    que `gen-assets.py` FORJA se llaman `icono-*` — como los nombran también
    `tray.py` y `index.html`. Resultado medido en macOS y en Windows: ningún icono
    se forjaba nunca, ni el `.icns` ni el `.ico`, y el `.app` nacía sin cara. Se
    declaraba con honestidad («el .app funciona igual») y era igual de falso: mi
    Regla 1 dice que un nombre se VERIFICA, no se adivina.
    """

    def test_todo_icono_nombrado_existe_en_el_disco(self):
        import re
        faltan = []
        for f in ("install-app.py", "instalar-app.py", "tray.py", "bandeja.py",
                  os.path.join("static", "index.html")):
            ruta = os.path.join(AQUI, f)
            if not os.path.exists(ruta):
                continue                      # cada edición trae sus nombres
            src = io.open(ruta, encoding="utf-8").read()
            for m in re.finditer(r'["\'/]([a-z0-9\-]*icon[a-z0-9\-]*\.png)', src):
                png = os.path.join(AQUI, "static", m.group(1))
                if not os.path.isfile(png):
                    faltan.append("%s -> static/%s" % (f, m.group(1)))
        self.assertEqual(faltan, [],
                         "el código nombra iconos que no existen: %s" % faltan)

    def test_el_generador_forja_los_que_el_instalador_pide(self):
        """Y no basta con que existan hoy: quien los FORJA tiene que escribir esos
        mismos nombres, o el próximo `gen-assets.py` los deja huérfanos otra vez."""
        gen = os.path.join(AQUI, "gen-assets.py")
        if not os.path.exists(gen):
            self.skipTest("el generador no viaja en esta disposición")
        forjados = io.open(gen, encoding="utf-8").read()
        for app in ("install-app.py", "instalar-app.py"):
            ruta = os.path.join(AQUI, app)
            if not os.path.exists(ruta):
                continue
            src = io.open(ruta, encoding="utf-8").read()
            import re
            for m in re.finditer(r'"static", "([a-z0-9\-]*icon[a-z0-9\-]*\.png)"', src):
                self.assertIn(m.group(1), forjados,
                              "%s pide `%s` y el generador no lo forja" % (app, m.group(1)))


if __name__ == "__main__":
    unittest.main(verbosity=2)
