#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""Red de pruebas del OJO — stdlib puro. Levanta el servidor real contra el
cuerpo vivo y verifica la puerta, las acciones y los endpoints.

    python3 test_eye.py
"""
import os, sys, json, time, subprocess, urllib.request, urllib.error, unittest

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
        cls.proc = subprocess.Popen(
            [sys.executable, os.path.join(AQUI, "server.py"), "--sin-navegador"],
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
        self.assertIn("desconocida", cuerpo)

    def test_inyeccion_en_el_argumento_rebota(self):
        h = dict(self.galleta); h["X-Ojo-Accion"] = "1"
        _c, cuerpo = _pedir(self.base + "/api/accion",
                            json.dumps({"accion": "saciar", "arg": "1; rm -rf /"}).encode(), h)
        self.assertIn("inaceptable", cuerpo)

    def test_todos_los_endpoints_responden(self):
        for r in ("/api/pulso", "/api/fallas", "/api/territorios", "/api/grafo",
                  "/api/linea", "/api/tiempo", "/api/actos", "/api/notas", "/api/salud"):
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


if __name__ == "__main__":
    unittest.main(verbosity=2)
