// Dibujo ÚNICO de las tareas en segundo plano (panel y ventana despegada lo comparten).
// Las que siguen corriendo, arriba; las terminadas (hecho, error, detenida), dentro de
// un desplegable «Finalizadas» CERRADO que recuerda si el usuario lo abrió.
const TASK_KIND = { turn: "▶", engine: "⚙", subagent: "⤷", compact: "🗜", lsp: "⌁", embedder: "🧭", download: "⬇" };
const taskEsc = (s) => String(s).replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]);
// «↑ 45,2k · ↓ 1,3k tok»: entrada (lo que leyó) y salida (lo que generó)
function taskTok(n) {
  if (n < 1000) return String(n);
  const k = (n / 1000).toFixed(1).replace(/\.0$/, "");
  return (typeof LANG !== "undefined" && LANG === "en" ? k : k.replace(".", ",")) + "k";
}
function taskTokens(t) {
  const u = t.tokens;
  if (!u) return "";
  const tip = `${tt("tokIn")}: ${u.in}${u.cached ? ` (${tt("tokCached")}: ${u.cached})` : ""} · ${tt("tokOut")}: ${u.out}${u.exact ? "" : " ≈"}`;
  return `<span class="tk" title="${taskEsc(tip)}">↑ ${taskTok(u.in)} · ↓ ${taskTok(u.out)} tok</span>`;
}
function taskElapsed(t) {
  const s = Math.round(((t.t1 || Date.now()) - t.t0) / 1000);
  return s < 60 ? s + " s" : Math.floor(s / 60) + " min " + (s % 60) + " s";
}
// C1: los procesos del chat (servidores, builds, instaladores): vivos con su URL y un
// botón para pararlos; los terminados que el agente aún no leyó, con su salida.
function procStamp(procs) {
  const now = Date.now();
  (procs || []).forEach((p) => { p._t0 = now - (p.secs || 0) * 1000; });
  return procs || [];
}
function procSecs(p) {
  const s = Math.round((Date.now() - (p._t0 || Date.now())) / 1000);
  return s < 60 ? s + " s" : s < 3600 ? Math.floor(s / 60) + " min" : Math.floor(s / 3600) + " h " + Math.floor((s % 3600) / 60) + " min";
}
function drawProcs(procs) {
  if (!procs || !procs.length) return "";
  const cmd = (c) => (c.length > 90 ? c.slice(0, 90) + "…" : c);
  const one = (p) => {
    // F8.7: en la ventana principal, «Ver aquí» la abre en la pestaña Vista (la despegada no tiene panel)
    const ver = (u) => (window.gotvVista ? ` <button type="button" class="pver" data-url="${taskEsc(u)}">${taskEsc(tt("vsVerAqui"))}</button>` : "");
    const urls = (p.urls || []).map((u) => `<a class="plink" data-url="${taskEsc(u)}">${taskEsc(u)}</a>${ver(u)}`).join(" ");
    const st = p.alive ? `${tt("procLive")} · ${procSecs(p)}` : taskEsc(p.exit || tt("done"));
    const btn = `<button class="pstop" data-proc="${p.id}" title="${taskEsc(tt(p.alive ? "procStopTip" : "procDismissTip"))}">${taskEsc(tt(p.alive ? "procStop" : "procDismiss"))}</button>`;
    return `<div class="task proc ${p.alive ? "running" : "done"}" title="${taskEsc(p.screen || "")}"><span class="k">▣</span><span class="tt">#${p.id} ${taskEsc(p.name)}${p.tty ? " ⌨" : ""}</span><span class="st">${st}</span><span class="dd"><code>${taskEsc(cmd(p.cmd))}</code>${urls ? "<br>" + urls : ""}${btn}</span></div>`;
  };
  return `<div class="proc-head">${taskEsc(tt("procs"))} · ${procs.length}</div>` + procs.map(one).join("");
}
let finOpen = false;
function drawTasks(box, tasks, procs) {
  if (!box) return;
  const ph = drawProcs(procs);
  if (!box.dataset.procWired) {
    box.dataset.procWired = "1";
    box.addEventListener("click", (e) => {
      const b = e.target.closest(".pstop");
      const a = e.target.closest("a.plink");
      if (b) {
        b.disabled = true;
        window.__TAURI__.core.invoke("proc_stop", { id: Number(b.dataset.proc) }).catch(() => (b.disabled = false));
      } else if (a) {
        window.__TAURI__.core.invoke("open_link", { url: a.dataset.url });
      }
    });
  }
  if (!tasks.length) { box.innerHTML = ph + `<div class="task-none">${taskEsc(tt("noTasks"))}</div>`; return; }
  const one = (t) => `<div class="task ${t.state}"><span class="k">${TASK_KIND[t.kind] || "•"}</span><span class="tt">${taskEsc(t.title || t.kind)}</span><span class="st">${taskEsc(tt(t.state))} · ${taskElapsed(t)}</span><span class="dd">${taskEsc(t.detail || "")}${taskTokens(t)}</span></div>`;
  const live = tasks.filter((t) => t.state === "running");
  const fin = tasks.filter((t) => t.state !== "running");
  box.innerHTML = ph +
    (live.length ? live.map(one).join("") : `<div class="task-none">${taskEsc(tt("noRunning"))}</div>`) +
    (fin.length ? `<details class="task-fin"${finOpen ? " open" : ""}><summary>${taskEsc(tt("finished"))} · ${fin.length}</summary>${fin.map(one).join("")}</details>` : "");
  const d = box.querySelector(".task-fin");
  if (d) d.ontoggle = () => (finOpen = d.open);
}
