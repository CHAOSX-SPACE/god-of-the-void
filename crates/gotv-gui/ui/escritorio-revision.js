// F16.5 · La tarjeta de `/revisar`: hallazgos con archivo, línea y el texto REAL de esa línea (validados en
// Rust contra el disco). «Arreglar éste» manda `/arreglar N` (la orden la arma Rust con lo guardado).
(() => {
  const inv = (c, a) => window.__TAURI__.core.invoke(c, a);
  const h = (tag, cls, txt) => { const e = document.createElement(tag); if (cls) e.className = cls; if (txt != null) e.textContent = txt; return e; };

  function tarjeta(m) {
    const t = h("div", "rev-card");
    t.appendChild(h("div", "rev-tit", (m.tipo === "seguridad" ? "🛡 " + tt("revSeguridad") : "🔎 " + tt("revTitulo")) + ` · ${m.hallazgos.length}`));
    if (!m.hallazgos.length) t.appendChild(h("div", "rev-vacio", "✅ " + tt("revSin")));
    for (const x of m.hallazgos) {
      const f = h("div", "rev-h sev-" + (x.severidad || "media"));
      const cab = h("div", "rev-cab");
      cab.appendChild(h("span", "rev-sev", x.severidad || "media"));
      cab.appendChild(h("code", "rev-loc", `${x.archivo}:${x.linea}`));
      f.appendChild(cab);
      f.appendChild(h("div", "rev-res", x.resumen));
      f.appendChild(h("pre", "rev-cod", `${x.linea} │ ${x.codigo}`));
      if (x.como_falla) f.appendChild(h("div", "rev-falla", `${tt("revComoFalla")}: ${x.como_falla}`));
      const b = h("button", "btn sm rev-arreglar", tt("revArreglar"));
      b.type = "button";
      b.onclick = () => {
        b.disabled = true;
        inv("send", { task: `/arreglar ${x.id}`, mac: false, perfil: null, rebobinar: null, pro: null }).catch((e) => { b.disabled = false; if (typeof toast === "function") toast("✗ " + e); });
        if (typeof setBusy === "function") setBusy(true);
      };
      f.appendChild(b);
      t.appendChild(f);
    }
    if (m.descartados && m.descartados.length) t.appendChild(h("div", "rev-desc", `⚠ ${m.descartados.length} ${tt("revDescartados")}: ${m.descartados.join("; ")}`));
    if (m.sin_json) t.appendChild(h("div", "rev-desc", "⚠ " + tt("revSinJson")));
    const log = document.getElementById("log");
    if (log) log.appendChild(t);
    t.scrollIntoView({ block: "nearest" });
  }

  window.__TAURI__.event.listen("gotv", (ev) => { if (ev.payload && ev.payload.kind === "revision") tarjeta(ev.payload); });
  window.gotvRevision = { tarjeta };
})();
