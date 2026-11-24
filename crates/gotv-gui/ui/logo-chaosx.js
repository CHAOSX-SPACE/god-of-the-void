// GOD OF THE VOID · el logo CHAOS-X, vivo, en la cabecera de la barra lateral.
//
// Fiel al logo (`logo-chaosx.png`, 651×383): las letras salen de SU imagen, píxel a píxel
// (componentes conexos de la tinta, con el antialias de sus bordes), y la X es lo que el logo
// dibuja — UNA barra «\» sólida y el trazo «/» hecho de anillos orbitales con una estrella en el
// cruce: aquí, una GALAXIA espiral de canto. Medido en la imagen: la barra va de (347–406, y 102)
// a (518–574, y 306); la galaxia de (364, 306) a (558, 102) con su núcleo en la estrella (464, 203);
// el «.» en (384, 222).
//
// Nacimiento (≈5 s): se enciende el núcleo con sus picos de difracción → la galaxia se despliega
// en espiral (rotación diferencial: el centro gira más rápido) → un láser traza la «\» y la barra
// se materializa → el núcleo dispara partículas que viajan en arco y CONSTRUYEN cada letra (S, O,
// A, H, C); cada letra se solidifica tras un escáner y pasa un brillo especular → la luna (el «.»)
// sale del núcleo y se aparca. Armado: la galaxia sigue girando (30 cps) 5 min → la galaxia se
// TRAGA las letras en espiral y colapsa en el núcleo (≈2,6 s) → 10 s → otra vez.
// «Reducir movimiento»: un cuadro quieto y armado. Ventana oculta: sin cuadros.
(() => {
  const NACE = 5000, MUERE = 2600, ARMADO = 5 * 60 * 1000, VACIO = 10 * 1000;
  // lo que se ve del logo (en píxeles de la imagen)
  // el encuadre: el logo MÁS el aire para sus brillos (halos de las letras, picos de la estrella, nebulosa),
  // para que nada se corte en el borde
  const VISTA = { x: 26, y: 68, w: 578, h: 268 };
  // el encuadre de la X sola (centro de la app): la barra «\», la galaxia y su núcleo, con aire
  const VISTA_X = { x: 322, y: 62, w: 284, h: 284 };
  const NUCLEO = [464, 203];
  const PUNTO = [384, 222];
  const BARRA = [[347, 102], [406, 102], [574, 306], [518, 306]];
  const GAL = { ang: Math.atan2(102 - 306, 558 - 364), R: Math.hypot(558 - 364, 306 - 102) / 2 + 4, inclina: 0.27 };
  const ESTRELLAS = 2400;
  // cuánto se nota la galaxia (el Portador: «un 50 % más visible»): brillo de estrellas y nebulosa ×1,5;
  // lo que ya llega al tope de brillo gana tamaño (√1,5) para que el aumento se vea igual
  const VISIBLE = 1.5;

  const clamp01 = (v) => Math.max(0, Math.min(1, v));
  const fase = (t, a, b) => clamp01((t - a) / (b - a));
  const salida = (u) => 1 - Math.pow(1 - u, 3);
  const vaiven = (u) => (u < 0.5 ? 4 * u * u * u : 1 - Math.pow(-2 * u + 2, 3) / 2);
  // azar con semilla: la misma galaxia en cada arranque
  let semilla = 7;
  const azar = () => ((semilla = (semilla * 16807) % 2147483647) - 1) / 2147483646;
  const gauss = () => { let s = 0; for (let i = 0; i < 4; i++) s += azar(); return (s - 2) / 0.58; };

  // las dos paletas: oscuro = luz sobre el vacío (mezcla aditiva); claro = la MISMA animación en
  // negativo sobre el fondo de la app (tinta índigo, metal pavonado, mezcla normal)
  const PALETAS = {
    oscuro: {
      metal: ["#fbfbff", "#d6d5e2", "#a9a7ba", "#cfcedb", "#8c8a9f"], cepillo: "30,28,48", luzBisel: "rgba(255,255,255,.9)", sombraBisel: "rgba(22,20,40,.75)",
      circuito: "rgba(110,95,240,.55)", via: "rgba(150,140,255,.85)", halo: "rgba(138,125,243,.95)",
      mezcla: "lighter", estrellas: ["255,224,180", "214,214,255", "170,160,255"],
      nebulosa: [[0, "255,236,210", 0.95], [0.12, "230,200,255", 0.55], [0.45, "138,125,243", 0.34], [0.8, "90,80,220", 0.16], [1, "60,50,180", 0]],
      nucleo: [[0, "255,255,255", 1], [0.18, "255,240,225", 0.9], [0.45, "170,150,255", 0.35], [1, "120,100,255", 0]],
      pico: ["200,190,255", "255,255,255"], chispa: "rgba(200,190,255,1)", laser: "rgba(200,190,255,.95)", escaner: "rgba(190,180,255,1)",
    },
    claro: {
      metal: ["#5f5d70", "#33313f", "#17161f", "#2d2b39", "#0c0b12"], cepillo: "255,255,255", luzBisel: "rgba(255,255,255,.55)", sombraBisel: "rgba(0,0,0,.6)",
      circuito: "rgba(150,135,255,.7)", via: "rgba(185,175,255,.95)", halo: "rgba(91,75,224,.75)",
      mezcla: "source-over", estrellas: ["120,70,24", "38,32,110", "88,68,205"],
      nebulosa: [[0, "48,30,120", 0.6], [0.12, "80,60,190", 0.4], [0.45, "120,105,235", 0.2], [0.8, "150,140,240", 0.09], [1, "170,160,245", 0]],
      // la estrella BLANCA, como en el logo (blanca sobre la X negra): resalta dentro de la barra
      nucleo: [[0, "255,255,255", 1], [0.16, "246,243,255", 0.95], [0.32, "120,105,235", 0.5], [1, "150,140,240", 0]],
      pico: ["91,75,224", "255,255,255"], chispa: "rgba(70,55,190,1)", laser: "rgba(74,57,212,.95)", escaner: "rgba(91,75,224,1)",
    },
  };
  const temaClaro = () => document.documentElement.dataset.theme === "light";

  function lienzo(w, h) { const c = document.createElement("canvas"); c.width = Math.max(1, Math.ceil(w)); c.height = Math.max(1, Math.ceil(h)); return c; }

  // ── las letras de la imagen: máscaras con el antialias de sus bordes
  function letrasDe(img) {
    const W = img.width, H = img.height;
    const c = lienzo(W, H), x = c.getContext("2d", { willReadFrequently: true });
    x.drawImage(img, 0, 0);
    const d = x.getImageData(0, 0, W, H).data;
    const cobertura = new Float32Array(W * H);
    for (let i = 0; i < W * H; i++) cobertura[i] = (d[i * 4 + 3] / 255) * (1 - (d[i * 4] + d[i * 4 + 1] + d[i * 4 + 2]) / 765);
    const lab = new Int32Array(W * H).fill(-1);
    const comps = [];
    for (let i = 0; i < W * H; i++) {
      if (cobertura[i] < 0.5 || lab[i] >= 0) continue;
      const id = comps.length, pila = [i];
      let x0 = W, y0 = H, x1 = 0, y1 = 0, n = 0;
      lab[i] = id;
      while (pila.length) {
        const j = pila.pop(), px = j % W, py = (j / W) | 0;
        n++; if (px < x0) x0 = px; if (px > x1) x1 = px; if (py < y0) y0 = py; if (py > y1) y1 = py;
        for (let dy = -1; dy <= 1; dy++) for (let dx = -1; dx <= 1; dx++) {
          const qx = px + dx, qy = py + dy;
          if (qx < 0 || qy < 0 || qx >= W || qy >= H) continue;
          const q = qy * W + qx;
          if (cobertura[q] >= 0.5 && lab[q] < 0) { lab[q] = id; pila.push(q); }
        }
      }
      comps.push({ id, n, x0, y0, x1, y1 });
    }
    // las letras: blobs de la altura de las mayúsculas, a la izquierda de la X
    const letras = comps.filter((k) => k.n > 600 && k.x1 < 380 && k.y1 - k.y0 > 45 && k.y1 - k.y0 < 80).sort((a, b) => a.x0 - b.x0);
    return letras.map((k) => {
      const pad = 3, x0 = k.x0 - pad, y0 = k.y0 - pad, w = k.x1 - k.x0 + 1 + 2 * pad, h = k.y1 - k.y0 + 1 + 2 * pad;
      const m = lienzo(w, h), mx = m.getContext("2d");
      const im = mx.createImageData(w, h);
      const puntos = [];
      for (let yy = 0; yy < h; yy++) for (let xx = 0; xx < w; xx++) {
        const px = x0 + xx, py = y0 + yy;
        if (px < 0 || py < 0 || px >= W || py >= H) continue;
        const j = py * W + px;
        let mio = lab[j] === k.id;
        if (!mio && cobertura[j] > 0.02) for (let dy = -1; dy <= 1 && !mio; dy++) for (let dx = -1; dx <= 1; dx++) { const q = (py + dy) * W + (px + dx); if (lab[q] === k.id) { mio = true; break; } }
        if (!mio) continue;
        const o = (yy * w + xx) * 4;
        im.data[o] = im.data[o + 1] = im.data[o + 2] = 255;
        im.data[o + 3] = Math.round(cobertura[j] * 255);
        if (cobertura[j] > 0.6) puntos.push([px, py]);
      }
      mx.putImageData(im, 0, 0);
      return { x0, y0, w, h, mascara: m, puntos };
    });
  }

  // ── el metal tecnológico de una forma (máscara): titanio cepillado, bisel, escaneo y circuito
  function metal(mascara, w, h, esc, semillaCircuito, P) {
    const W = Math.ceil(w * esc), H = Math.ceil(h * esc);
    const forma = lienzo(W, H), fx = forma.getContext("2d");
    fx.imageSmoothingQuality = "high";
    fx.drawImage(mascara, 0, 0, W, H);
    const c = lienzo(W, H), x = c.getContext("2d");
    const g = x.createLinearGradient(0, 0, 0, H);
    [0, 0.42, 0.5, 0.62, 1].forEach((o, k) => g.addColorStop(o, P.metal[k]));
    x.fillStyle = g; x.fillRect(0, 0, W, H);
    // cepillado y líneas de escaneo
    for (let yy = 0; yy < H; yy += Math.max(1, esc * 1.6)) { x.fillStyle = `rgba(${P.cepillo},${0.05 + 0.04 * ((yy * 7) % 3)})`; x.fillRect(0, yy, W, Math.max(0.6, esc * 0.35)); }
    // pistas de circuito grabadas (ángulos rectos, con su vía)
    let s = semillaCircuito;
    const r = () => ((s = (s * 48271) % 2147483647) / 2147483647);
    x.strokeStyle = P.circuito; x.lineWidth = Math.max(0.7, esc * 0.55); x.fillStyle = P.via;
    for (let i = 0; i < 4; i++) {
      let px = r() * W, py = r() * H;
      x.beginPath(); x.moveTo(px, py);
      for (let p = 0; p < 3; p++) { if (p % 2) py += (r() - 0.5) * H * 0.7; else px += (r() - 0.5) * W * 0.8; x.lineTo(px, py); }
      x.stroke();
      x.beginPath(); x.arc(px, py, Math.max(0.8, esc * 0.7), 0, Math.PI * 2); x.fill();
    }
    x.globalCompositeOperation = "destination-in";
    x.drawImage(forma, 0, 0);
    // bisel: luz arriba-izquierda, sombra abajo-derecha, sólo dentro de la forma
    const bisel = (dx, dy, color) => {
      const b = lienzo(W, H), bx = b.getContext("2d");
      bx.drawImage(forma, 0, 0);
      bx.globalCompositeOperation = "destination-out"; bx.drawImage(forma, dx, dy);
      bx.globalCompositeOperation = "source-in"; bx.fillStyle = color; bx.fillRect(0, 0, W, H);
      x.globalCompositeOperation = "source-atop"; x.drawImage(b, 0, 0);
    };
    bisel(esc * 1.3, esc * 1.3, P.luzBisel);
    bisel(-esc * 1.3, -esc * 1.3, P.sombraBisel);
    x.globalCompositeOperation = "source-over";
    // halo energizado (detrás)
    const halo = lienzo(W + 24 * esc, H + 24 * esc), hx = halo.getContext("2d");
    hx.shadowColor = P.halo; hx.shadowBlur = 7 * esc;
    hx.drawImage(forma, 12 * esc, 12 * esc);
    return { metal: c, forma, halo, W, H, pad: 12 * esc };
  }

  // `{ solo: "x" }`: sólo la X animada (la barra y la galaxia), para el centro de la app
  function montar(caja, opciones = {}) {
    if (!caja || caja.dataset.logo) return;
    caja.dataset.logo = "1";
    const soloX = opciones.solo === "x";
    // `{ solo: "x", icono: 1024 }`: la X dentro de la plantilla de icono de macOS (cuadrado de
    // TAM px, sin ciclo propio: quien la usa pide cada cuadro con `caja._logo.cuadro(t)`)
    const icono = soloX && opciones.icono > 0 ? Math.round(opciones.icono) : 0;
    const V = soloX ? VISTA_X : VISTA;
    const c = document.createElement("canvas");
    c.className = soloX ? "logo-vivo logo-x" : "logo-vivo";
    c.style.aspectRatio = `${V.w} / ${V.h}`; // la página da el tamaño; el lienzo no lo impone
    c.setAttribute("role", "img");
    c.setAttribute("aria-label", soloX ? "X · GOD OF THE VOID" : "CHAOS-X · GOD OF THE VOID");
    caja.appendChild(c);
    // la X sola se dibuja por geometría: no hace falta la imagen
    if (soloX) { try { arrancar(caja, c, null, true, icono); } catch (e) { console.error("logo:", e); } return; }
    const img = new Image();
    // onload (no decode(): con la ventana oculta, decode() se aplaza y el logo no arrancaba)
    img.onload = () => { try { arrancar(caja, c, img, false); } catch (e) { console.error("logo:", e); } };
    img.onerror = () => console.error("logo: no pude leer logo-chaosx.png");
    img.src = "logo-chaosx.png";
  }

  // la silueta del icono de macOS: superelipse (esquinas continuas, no un rectángulo redondeado)
  // de 824/1024 del lienzo, centrada — la plantilla de Apple para los iconos de la app
  function siluetaIcono(S) {
    const k = S / 1024, a = 412 * k, cx = S / 2, cy = S / 2, n = 5, p = new Path2D();
    for (let i = 0; i <= 360; i++) {
      const u = (i / 360) * Math.PI * 2, co = Math.cos(u), si = Math.sin(u);
      const x = cx + Math.sign(co) * Math.pow(Math.abs(co), 2 / n) * a;
      const y = cy + Math.sign(si) * Math.pow(Math.abs(si), 2 / n) * a;
      i ? p.lineTo(x, y) : p.moveTo(x, y);
    }
    p.closePath();
    return p;
  }

  function arrancar(caja, c, img, soloX, icono = 0) {
    const ctx = c.getContext("2d");
    const VISTA_ = soloX ? VISTA_X : VISTA;
    // la X sola nace antes (sin letras ni luna): núcleo → galaxia → barra
    const N = soloX ? 2800 : NACE;
    const BRILLO_BARRA = soloX ? [2150, 2750] : [4300, 4900];
    let dpr = 1, esc = 1, CW = 0, CH = 0, OX = 0, OY = 0, fondo = null, silueta = null;
    let letras = [], barra = null, brillo = null, galaxiaGlow = null, P = PALETAS.oscuro;
    const L = soloX ? [] : letrasDe(img);
    // del espacio de la imagen al del lienzo (px de dispositivo); el icono la centra en su silueta
    const aX = (x) => (x - VISTA_.x) * esc + OX, aY = (y) => (y - VISTA_.y) * esc + OY;

    // las estrellas: radio (con dos anillos densos donde el logo dibuja sus anillos), brazo espiral,
    // color por población (azules en los brazos, doradas al centro), brillo y titileo
    semilla = 7;
    const est = [];
    for (let i = 0; i < ESTRELLAS; i++) {
      const tipo = azar();
      let r;
      if (tipo < 0.3) r = 0.74 + gauss() * 0.035; // anillo interior del logo
      else if (tipo < 0.55) r = 0.93 + gauss() * 0.03; // anillo exterior
      else if (tipo < 0.8) r = Math.pow(azar(), 0.55) * 1.02; // disco
      else r = Math.abs(gauss()) * 0.16; // bulbo
      r = Math.max(0.02, Math.min(1.06, r));
      const brazo = azar() < 0.5 ? 0 : Math.PI;
      const espiral = tipo >= 0.55 && tipo < 0.8 ? brazo + Math.log(r + 0.05) * 2.3 + gauss() * 0.35 : azar() * Math.PI * 2;
      const calor = r < 0.2 ? 0 : tipo < 0.55 ? 2 : azar() < 0.6 ? 1 : 2;
      est.push({ r, a0: espiral, w: 0.00042 / Math.sqrt(r + 0.08), t: azar() * 6.28, b: Math.min(1, 0.5 + azar() * 0.5 + (tipo < 0.55 ? 0.18 : 0)), s: azar() < 0.06 ? 1.6 : azar() < 0.3 ? 1.05 : 0.7, calor, nace: azar() });
    }

    // partículas que construyen las letras (desde el núcleo, en arco)
    const chispas = [];
    const medir = () => {
      P = temaClaro() ? PALETAS.claro : PALETAS.oscuro;
      // el tamaño lo decide la página (CSS: 100 % de su sitio y la proporción del encuadre); aquí sólo
      // se lee para dibujar a la densidad de la pantalla. Antes se tomaba `clientWidth` de la cabecera,
      // que INCLUYE su relleno: el lienzo quedaba 15 px más ancho que su hueco y se desbordaba.
      if (icono) {
        // el icono: siempre la paleta del vacío (el Dock no sigue el tema de la app), la X al 88 %
        // de la silueta; `dpr` crece con el tamaño para que las estrellas guarden su peso
        P = PALETAS.oscuro;
        CW = CH = icono; c.width = CW; c.height = CH;
        const contenido = 824 * (icono / 1024) * 0.88;
        esc = contenido / VISTA_.w; OX = OY = (icono - contenido) / 2; dpr = contenido / 210; // estrellas finas: a este tamaño un cuadro grueso se ve de bloques
        silueta = siluetaIcono(icono);
        fondo = lienzo(icono, icono);
        const fx = fondo.getContext("2d");
        fx.save(); fx.clip(silueta);
        const fg = fx.createLinearGradient(0, icono * 0.1, 0, icono * 0.9);
        fg.addColorStop(0, "#1a1729"); fg.addColorStop(0.55, "#0c0b14"); fg.addColorStop(1, "#050508");
        fx.fillStyle = fg; fx.fillRect(0, 0, icono, icono);
        // un leve resplandor índigo detrás del núcleo: profundidad, no decoración
        const nx = (NUCLEO[0] - VISTA_.x) * esc + OX, ny = (NUCLEO[1] - VISTA_.y) * esc + OY;
        const rg = fx.createRadialGradient(nx, ny, 0, nx, ny, icono * 0.42);
        rg.addColorStop(0, "rgba(98,82,230,.22)"); rg.addColorStop(1, "rgba(98,82,230,0)");
        fx.fillStyle = rg; fx.fillRect(0, 0, icono, icono);
        fx.restore();
      } else {
        const ancho = c.getBoundingClientRect().width || 236;
        dpr = Math.min(2, window.devicePixelRatio || 1);
        CW = Math.max(1, Math.round(ancho * dpr)); CH = Math.max(1, Math.round((ancho * VISTA_.h / VISTA_.w) * dpr));
        c.width = CW; c.height = CH;
        esc = CW / VISTA_.w;
      }
      letras = L.map((l, i) => ({ ...l, i, ...metal(l.mascara, l.w, l.h, esc, 1234 + i * 977, P) }));
      // la barra «\»: un polígono con el mismo metal
      const bx0 = 340, by0 = 96, bw = 242, bh = 216;
      const bm = lienzo(bw, bh), bmx = bm.getContext("2d");
      bmx.fillStyle = "#fff"; bmx.beginPath();
      BARRA.forEach(([x, y], k) => (k ? bmx.lineTo(x - bx0, y - by0) : bmx.moveTo(x - bx0, y - by0)));
      bmx.closePath(); bmx.fill();
      barra = { x0: bx0, y0: by0, w: bw, h: bh, ...metal(bm, bw, bh, esc, 4242, P) };
      // brillo especular: una banda diagonal
      brillo = lienzo(60 * esc, 300 * esc);
      const bx = brillo.getContext("2d"), bg = bx.createLinearGradient(0, 0, brillo.width, 0);
      bg.addColorStop(0, "rgba(255,255,255,0)"); bg.addColorStop(0.5, "rgba(255,255,255,.85)"); bg.addColorStop(1, "rgba(255,255,255,0)");
      bx.fillStyle = bg; bx.fillRect(0, 0, brillo.width, brillo.height);
      // la nebulosa del disco (estática: se pinta una vez)
      const R = GAL.R * esc;
      galaxiaGlow = lienzo(R * 2.6, R * 2.6);
      const gx = galaxiaGlow.getContext("2d");
      gx.translate(galaxiaGlow.width / 2, galaxiaGlow.height / 2); gx.rotate(GAL.ang); gx.scale(1, GAL.inclina);
      const ng = gx.createRadialGradient(0, 0, 0, 0, 0, R * 1.15);
      for (const [o, rgb, a] of P.nebulosa) ng.addColorStop(o, `rgba(${rgb},${o === 0 ? a : Math.min(1, a * VISIBLE)})`);
      gx.fillStyle = ng; gx.beginPath(); gx.arc(0, 0, R * 1.15, 0, Math.PI * 2); gx.fill();
      chispas.length = 0;
      semilla = 99;
      for (const l of letras) {
        const n = Math.min(170, l.puntos.length);
        for (let k = 0; k < n; k++) {
          const p = l.puntos[Math.floor(azar() * l.puntos.length)];
          chispas.push({ l: l.i, p, curva: (azar() - 0.5) * 160, d: azar() * 0.35 });
        }
      }
    };
    medir();
    // al cambiar el tamaño (ventana, barra) o la densidad (otra pantalla): se vuelve a medir y a pintar
    const remedir = () => { const a = c.getBoundingClientRect().width; const d = Math.min(2, window.devicePixelRatio || 1); if (a && (Math.round(a * d) !== CW || d !== dpr)) { medir(); pinta(T, 0, performance.now()); } };
    // (el icono tiene tamaño y paleta fijos y no vive en la página: nada que vigilar)
    const ro = icono ? null : new ResizeObserver(() => (c.isConnected ? remedir() : apagar()));
    ro?.observe(c);
    const vigilaDensidad = () => matchMedia(`(resolution: ${window.devicePixelRatio || 1}dppx)`).addEventListener("change", () => { remedir(); vigilaDensidad(); }, { once: true });
    if (!icono) vigilaDensidad();
    // cambiar de tema rehace el logo con la otra paleta (en claro: la misma animación en negativo)
    const mo = icono ? null : new MutationObserver(() => { if (!c.isConnected) return apagar(); medir(); pinta(T, 0, performance.now()); });
    mo?.observe(document.documentElement, { attributes: true, attributeFilter: ["data-theme"] });

    // ── el horario
    const letraIni = (i) => 2050 + (letras.length - 1 - i) * 260; // de la X hacia afuera: S, O, A, H, C
    const letraFin = (i) => letraIni(i) + 900;

    let T = 0;
    const galaxia = (t, delante, formacion, colapso) => {
      const cx = aX(NUCLEO[0]), cy = aY(NUCLEO[1]), R = GAL.R * esc;
      const ca = Math.cos(GAL.ang), sa = Math.sin(GAL.ang);
      for (let k = 0; k < 3; k++) {
        ctx.fillStyle = `rgb(${P.estrellas[k]})`;
        for (const s of est) {
          if (s.calor !== k) continue;
          // formación: cada estrella sale del núcleo a su radio; colapso: cae en espiral al centro
          const f = clamp01((formacion - s.nace * 0.55) / 0.45);
          if (f <= 0) continue;
          const caida = colapso > 0 ? clamp01((colapso - (1 - s.r) * 0.4) / 0.6) : 0;
          const r = s.r * salida(f) * (1 - caida);
          const a = s.a0 + s.w * t + (1 - f) * 2.2 + caida * 5;
          const sn = Math.sin(a);
          if ((sn > 0) !== delante) continue;
          const lx = Math.cos(a) * r * R, ly = sn * r * R * GAL.inclina;
          const x = cx + lx * ca - ly * sa, y = cy + lx * sa + ly * ca;
          const tw = 0.7 + 0.3 * Math.sin(t * 0.0021 + s.t * 7);
          const luz = s.b * tw * f * (1 - caida * 0.6) * VISIBLE;
          ctx.globalAlpha = Math.min(1, luz);
          const z = s.s * dpr * (0.8 + 0.4 * (1 - r)) * Math.sqrt(Math.max(1, luz));
          if (icono) { ctx.beginPath(); ctx.arc(x, y, z * 0.6, 0, Math.PI * 2); ctx.fill(); } // en grande: redondas
          else ctx.fillRect(x - z / 2, y - z / 2, z, z);
        }
      }
      ctx.globalAlpha = 1;
    };
    const nucleo = (t, encendido) => {
      if (encendido <= 0) return;
      const cx = aX(NUCLEO[0]), cy = aY(NUCLEO[1]);
      ctx.save();
      ctx.globalCompositeOperation = P.mezcla;
      const pulso = 0.92 + 0.08 * Math.sin(t * 0.0017);
      const r = 26 * esc * encendido * pulso;
      const g = ctx.createRadialGradient(cx, cy, 0, cx, cy, r);
      for (const [o, rgb, a] of P.nucleo) g.addColorStop(o, `rgba(${rgb},${a})`);
      ctx.fillStyle = g; ctx.beginPath(); ctx.arc(cx, cy, r, 0, Math.PI * 2); ctx.fill();
      // la estrella de cuatro puntas del logo + picos de difracción
      const largo = 30 * esc * encendido * pulso, ancho = 2.2 * esc;
      for (const [dx, dy] of [[1, 0], [0, 1]]) {
        const pg = ctx.createLinearGradient(cx - dx * largo, cy - dy * largo, cx + dx * largo, cy + dy * largo);
        pg.addColorStop(0, `rgba(${P.pico[0]},0)`); pg.addColorStop(0.5, `rgba(${P.pico[1]},.95)`); pg.addColorStop(1, `rgba(${P.pico[0]},0)`);
        ctx.fillStyle = pg;
        ctx.beginPath();
        ctx.moveTo(cx - dx * largo, cy - dy * largo);
        ctx.lineTo(cx + dy * ancho, cy + dx * ancho);
        ctx.lineTo(cx + dx * largo, cy + dy * largo);
        ctx.lineTo(cx - dy * ancho, cy - dx * ancho);
        ctx.closePath(); ctx.fill();
      }
      ctx.restore();
    };
    const pintaMetal = (m, x0, y0, alfa, revela, energia, linea = true) => {
      if (alfa <= 0) return;
      const X = aX(x0), Y = aY(y0);
      if (energia > 0) { ctx.globalAlpha = energia * alfa; ctx.drawImage(m.halo, X - m.pad, Y - m.pad); }
      ctx.globalAlpha = alfa;
      if (revela >= 1) ctx.drawImage(m.metal, X, Y);
      else if (revela > 0 && !linea) ctx.drawImage(m.metal, 0, m.H - m.H * revela, m.W, m.H * revela, X, Y + m.H - m.H * revela, m.W, m.H * revela);
      else if (revela > 0) {
        // el escáner: de abajo hacia arriba, con su línea de luz
        const h = m.H * revela;
        ctx.drawImage(m.metal, 0, m.H - h, m.W, h, X, Y + m.H - h, m.W, h);
        ctx.globalAlpha = alfa * 0.9;
        ctx.fillStyle = P.escaner;
        ctx.fillRect(X - 1 * esc, Y + m.H - h - 0.6 * dpr, m.W + 2 * esc, 1.2 * dpr);
      }
      ctx.globalAlpha = 1;
    };
    const brilloSobre = (m, x0, y0, u) => {
      if (u <= 0 || u >= 1) return;
      const tmp = lienzo(m.W, m.H), tx = tmp.getContext("2d");
      tx.drawImage(m.forma, 0, 0);
      tx.globalCompositeOperation = "source-in";
      tx.translate(-brillo.width + (m.W + brillo.width * 2) * u, -m.H * 0.6); tx.rotate(0.35);
      tx.drawImage(brillo, 0, 0);
      ctx.save(); ctx.globalCompositeOperation = "lighter"; ctx.globalAlpha = 0.75;
      ctx.drawImage(tmp, aX(x0), aY(y0));
      ctx.restore();
    };

    // ── un cuadro: t = tiempo del nacimiento (0…N); muerte = 0…1 durante el colapso
    const pinta = (t, muerte = 0, ahora = performance.now()) => {
      T = t;
      ctx.clearRect(0, 0, CW, CH);
      if (icono) { ctx.drawImage(fondo, 0, 0); ctx.save(); ctx.clip(silueta); }
      const enc = salida(fase(t, 0, 700)) * (1 - fase(muerte, 0.75, 1));
      const formacion = vaiven(fase(t, 350, 2500));
      // nebulosa y estrellas de atrás
      ctx.save(); ctx.globalCompositeOperation = P.mezcla;
      ctx.globalAlpha = formacion * (1 - fase(muerte, 0.3, 0.95));
      ctx.drawImage(galaxiaGlow, aX(NUCLEO[0]) - galaxiaGlow.width / 2, aY(NUCLEO[1]) - galaxiaGlow.height / 2);
      ctx.restore();
      galaxia(ahora, false, formacion, muerte);
      // la barra «\»: un láser la traza y se materializa
      const ul = fase(t, 1500, 2050), um = fase(t, 1850, 2500);
      const barraAlfa = 1 - fase(muerte, 0.35, 0.8);
      if (ul > 0 && ul < 1) {
        const [a, b] = [[(347 + 406) / 2, 102], [(518 + 574) / 2, 306]];
        ctx.save(); ctx.globalCompositeOperation = P.mezcla;
        ctx.strokeStyle = P.laser; ctx.lineWidth = 1.4 * dpr; ctx.shadowColor = P.laser; ctx.shadowBlur = 8 * dpr;
        ctx.beginPath(); ctx.moveTo(aX(a[0]), aY(a[1])); ctx.lineTo(aX(a[0] + (b[0] - a[0]) * salida(ul)), aY(a[1] + (b[1] - a[1]) * salida(ul))); ctx.stroke();
        ctx.restore();
      }
      pintaMetal(barra, barra.x0, barra.y0, barraAlfa * clamp01(um * 3), salida(um), (1 - fase(t, 2500, 3600)) * 0.8 + 0.18, false);
      brilloSobre(barra, barra.x0, barra.y0, fase(t, BRILLO_BARRA[0], BRILLO_BARRA[1]));
      // las estrellas de delante (pasan por encima de la barra, como los anillos del logo)
      galaxia(ahora, true, formacion, muerte);
      nucleo(ahora, enc);
      // las letras: chispas desde el núcleo en arco → escáner → metal → brillo
      const [nx, ny] = NUCLEO;
      ctx.save(); ctx.globalCompositeOperation = P.mezcla; ctx.fillStyle = P.chispa;
      for (const ch of chispas) {
        const l = letras[ch.l];
        const u = fase(t, letraIni(ch.l) - 500 + ch.d * 400, letraIni(ch.l) + 250 + ch.d * 400);
        let x, y, a;
        if (muerte > 0) {
          // el colapso: de su sitio hacia el núcleo, girando
          const v = clamp01((muerte - ch.l * 0.04 - ch.d * 0.2) / 0.5);
          if (v <= 0 || v >= 1) continue;
          const ang = v * 4 + ch.d * 6, rad = (1 - v);
          x = nx + (ch.p[0] - nx) * rad + Math.cos(ang) * 12 * v * (1 - v);
          y = ny + (ch.p[1] - ny) * rad + Math.sin(ang) * 12 * v * (1 - v);
          a = Math.sin(Math.PI * v);
        } else {
          if (u <= 0 || u >= 1) continue;
          const e = salida(u);
          const mx = (nx + ch.p[0]) / 2, my = (ny + ch.p[1]) / 2 - 40 - ch.curva * 0.25;
          x = (1 - e) * (1 - e) * nx + 2 * (1 - e) * e * mx + e * e * ch.p[0];
          y = (1 - e) * (1 - e) * ny + 2 * (1 - e) * e * my + e * e * ch.p[1];
          a = Math.sin(Math.PI * u);
        }
        ctx.globalAlpha = a;
        const z = 1.15 * dpr;
        ctx.fillRect(aX(x) - z / 2, aY(y) - z / 2, z, z);
        if (l && muerte === 0) { ctx.globalAlpha = a * 0.25; ctx.fillRect(aX(x) - z * 1.5, aY(y) - z * 1.5, z * 3, z * 3); }
      }
      ctx.restore();
      for (const l of letras) {
        const ini = letraIni(l.i), fin = letraFin(l.i);
        const vivo = 1 - fase(muerte, 0.05 + l.i * 0.05, 0.4 + l.i * 0.06);
        pintaMetal(l, l.x0, l.y0, vivo * clamp01(fase(t, ini - 200, ini + 200) * 2), salida(fase(t, ini, fin)), (1 - fase(t, fin, fin + 1100)) * 0.9 + 0.12);
        brilloSobre(l, l.x0, l.y0, fase(t, 4200 + l.i * 70, 4800 + l.i * 70));
      }
      // la luna (el «.»): sale del núcleo en espiral y se aparca
      const ul2 = soloX ? 0 : fase(t, 3700, 4500); // la luna es del «CHAOS.»
      const luna = ul2 * (1 - fase(muerte, 0.2, 0.55));
      if (luna > 0) {
        const e = salida(ul2), giro = (1 - e) * 3.4;
        const dx = PUNTO[0] - nx, dy = PUNTO[1] - ny;
        const x = nx + (dx * Math.cos(giro) - dy * Math.sin(giro) * 0.45) * e, y = ny + (dx * Math.sin(giro) * 0.45 + dy * Math.cos(giro)) * e;
        const r = 5.2 * esc;
        const g = ctx.createRadialGradient(aX(x) - r * 0.35, aY(y) - r * 0.35, r * 0.1, aX(x), aY(y), r);
        g.addColorStop(0, "#ffffff"); g.addColorStop(0.45, "#c9c4f5"); g.addColorStop(1, "#4b3fb8");
        ctx.globalAlpha = clamp01(luna * 2);
        ctx.save(); ctx.shadowColor = "rgba(138,125,243,.9)"; ctx.shadowBlur = 6 * dpr;
        ctx.fillStyle = g; ctx.beginPath(); ctx.arc(aX(x), aY(y), r, 0, Math.PI * 2); ctx.fill();
        ctx.restore(); ctx.globalAlpha = 1;
      }
      if (icono) {
        ctx.restore();
        // el canto del icono: una línea de luz arriba que se apaga hacia abajo
        const cg = ctx.createLinearGradient(0, CH * 0.1, 0, CH * 0.9);
        cg.addColorStop(0, "rgba(255,255,255,.16)"); cg.addColorStop(0.5, "rgba(255,255,255,.04)"); cg.addColorStop(1, "rgba(255,255,255,.08)");
        ctx.save(); ctx.strokeStyle = cg; ctx.lineWidth = Math.max(1, CW / 512); ctx.stroke(silueta); ctx.restore();
      }
    };

    // ── el ciclo
    const quieto = matchMedia("(prefers-reduced-motion: reduce)");
    let cuadro = 0, reloj = 0, ultimo = 0;
    const cancelar = () => { cancelAnimationFrame(cuadro); clearTimeout(reloj); };
    // fuera de la página (el centro de la app se rehace a menudo): se apaga del todo — sin bucles ni
    // observadores vivos que sigan gastando
    const apagar = () => { cancelar(); ro?.disconnect(); mo?.disconnect(); quieto.removeEventListener("change", empezar); document.removeEventListener("visibilitychange", alVolver); };
    const correr = (dur, cuadroDe, alFin) => {
      const t0 = performance.now();
      const paso = (ahora) => {
        if (!c.isConnected) return apagar();
        const u = Math.min(1, (ahora - t0) / dur);
        cuadroDe(u, ahora);
        if (u < 1) cuadro = requestAnimationFrame(paso); else alFin();
      };
      cuadro = requestAnimationFrame(paso);
    };
    // armado: la galaxia sigue girando a ~30 cuadros por segundo; oculta, no se pinta
    const girar = (hasta, luego) => {
      const paso = (ahora) => {
        if (!c.isConnected) return apagar();
        if (ahora >= hasta) return luego();
        if (!document.hidden && ahora - ultimo > 33) { ultimo = ahora; pinta(N, 0, ahora); }
        cuadro = requestAnimationFrame(paso);
      };
      cuadro = requestAnimationFrame(paso);
    };
    const ciclo = () => {
      cancelar();
      correr(N, (u, ahora) => pinta(N * u, 0, ahora), () =>
        girar(performance.now() + ARMADO, () =>
          correr(MUERE, (u, ahora) => pinta(N, u, ahora), () => { ctx.clearRect(0, 0, CW, CH); reloj = setTimeout(() => (c.isConnected ? ciclo() : apagar()), VACIO); })));
    };
    const empezar = () => { cancelar(); if (quieto.matches) pinta(N, 0, 0); else ciclo(); };
    const alVolver = () => { if (!document.hidden && quieto.matches) pinta(N, 0, 0); };
    if (icono) pinta(N, 0, 0); // sin ciclo: el que lo usa pide los cuadros
    else {
      quieto.addEventListener("change", empezar);
      document.addEventListener("visibilitychange", alVolver);
      empezar();
    }
    // para revisar sin esperar 5 minutos (consola): document.getElementById("brand")._logo
    caja._logo = {
      nacer: () => { cancelar(); ciclo(); },
      cuadro: (t, muerte = 0, ahora = performance.now()) => { cancelar(); pinta(t, muerte, ahora); },
      tiempos: { NACE: N, MUERE, ARMADO, VACIO },
      lienzo: c,
    };
  }

  window.GotvLogo = { montar };
})();
