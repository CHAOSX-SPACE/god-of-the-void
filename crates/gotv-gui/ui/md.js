// Markdown de las respuestas: propio, pequeño y SEGURO. Primero se escapa TODO el
// HTML del modelo y después se da formato, así que ningún texto del modelo puede
// inyectar etiquetas. Soporta lo que los modelos escriben de verdad: títulos,
// negrita/cursiva/tachado, código en línea y en bloque (con «copiar»), listas
// (anidadas por sangría, con casillas), citas, tablas, separadores y enlaces http(s).
// Tolera el texto A MEDIO ESCRIBIR (streaming): un bloque de código sin cerrar se
// pinta como código hasta el final.
(function () {
  const esc = (s) => s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");

  function inline(s) {
    // 1) el código en línea se aparta para que nada de adentro tome formato
    const codes = [];
    s = s.replace(/`([^`\n]+)`/g, (_, c) => { codes.push(c); return `\u0000${codes.length - 1}\u0000`; });
    s = esc(s);
    // enlaces [texto](http…) y direcciones sueltas
    s = s.replace(/\[([^\]\n]+)\]\((https?:\/\/[^\s)]+)\)/g, (_, t, u) => `<a href="#" data-href="${u}" title="${u}">${t}</a>`);
    s = s.replace(/(^|[\s(])(https?:\/\/[^\s<)]+[^\s<).,;:!?])/g, (_, p, u) => `${p}<a href="#" data-href="${u}" title="${u}">${u}</a>`);
    // negrita sólo con ** (el __x__ en un agente de código casi siempre es Python: __init__)
    s = s.replace(/\*\*([^*\n]+?)\*\*/g, "<strong>$1</strong>");
    s = s.replace(/(^|[^*\w])\*([^*\s][^*\n]*?)\*(?!\w)/g, "$1<em>$2</em>");
    s = s.replace(/(^|[^\w])_([^_\s][^_\n]*?)_(?!\w)/g, "$1<em>$2</em>");
    s = s.replace(/~~([^~\n]+?)~~/g, "<del>$1</del>");
    return s.replace(/\u0000(\d+)\u0000/g, (_, i) => `<code>${esc(codes[+i])}</code>`);
  }

  const isTableSep = (l) => /^\s*\|?\s*:?-{2,}:?\s*(\|\s*:?-{2,}:?\s*)*\|?\s*$/.test(l);
  const cells = (l) => l.trim().replace(/^\|/, "").replace(/\|$/, "").split("|").map((c) => c.trim());
  const listRe = /^(\s*)([-*+]|\d{1,3}[.)])\s+(.*)$/;

  function list(lines, i) {
    // lista con anidación por sangría; devuelve [html, siguiente línea]
    const base = lines[i].match(listRe)[1].length;
    const ordered = /\d/.test(lines[i].match(listRe)[2]);
    let html = ordered ? "<ol>" : "<ul>";
    while (i < lines.length) {
      const m = lines[i].match(listRe);
      if (!m) {
        // continuación de un ítem (línea con sangría) o fin de la lista
        if (lines[i].trim() && /^\s+/.test(lines[i]) && html.endsWith("</li>")) {
          html = html.slice(0, -5) + "<br>" + inline(lines[i].trim()) + "</li>";
          i++;
          continue;
        }
        break;
      }
      const ind = m[1].length;
      if (ind < base) break;
      if (ind > base) {
        const [sub, next] = list(lines, i);
        html = html.endsWith("</li>") ? html.slice(0, -5) + sub + "</li>" : html + sub;
        i = next;
        continue;
      }
      let body = m[3];
      const box = body.match(/^\[([ xX])\]\s+(.*)$/);
      body = box ? `<span class="mdbox${box[1] === " " ? "" : " on"}"></span>${inline(box[2])}` : inline(body);
      html += `<li>${body}</li>`;
      i++;
    }
    return [html + (ordered ? "</ol>" : "</ul>"), i];
  }

  function render(src) {
    const lines = String(src || "").replace(/\r\n?/g, "\n").split("\n");
    let out = "", para = [];
    const flush = () => { if (para.length) { out += `<p>${para.map(inline).join("<br>")}</p>`; para = []; } };
    for (let i = 0; i < lines.length; ) {
      const l = lines[i];
      const fence = l.match(/^\s*(```+|~~~+)\s*([\w+#.-]*)\s*$/);
      if (fence) {
        flush();
        const code = [];
        i++;
        while (i < lines.length && !lines[i].trim().startsWith(fence[1])) code.push(lines[i++]);
        i++; // la valla de cierre (o el final, si aún se está escribiendo)
        out += `<div class="mdcode"><div class="mdbar"><span>${esc(fence[2] || "")}</span><button class="mdcopy" type="button">${typeof tt === "function" ? tt("copy") : "copy"}</button></div><pre><code>${esc(code.join("\n"))}</code></pre></div>`;
        continue;
      }
      if (!l.trim()) { flush(); i++; continue; }
      const h = l.match(/^(#{1,6})\s+(.*?)\s*#*\s*$/);
      if (h) { flush(); const n = Math.min(h[1].length + 2, 6); out += `<h${n}>${inline(h[2])}</h${n}>`; i++; continue; }
      if (/^\s*([-*_])(\s*\1){2,}\s*$/.test(l)) { flush(); out += "<hr>"; i++; continue; }
      if (/^\s*>/.test(l)) {
        flush();
        const q = [];
        while (i < lines.length && /^\s*>/.test(lines[i])) q.push(lines[i++].replace(/^\s*>\s?/, ""));
        out += `<blockquote>${render(q.join("\n"))}</blockquote>`;
        continue;
      }
      if (l.includes("|") && i + 1 < lines.length && isTableSep(lines[i + 1])) {
        flush();
        const head = cells(l);
        const align = cells(lines[i + 1]).map((c) => (c.startsWith(":") && c.endsWith(":") ? "center" : c.endsWith(":") ? "right" : ""));
        const td = (c, k, tag) => `<${tag}${align[k] ? ` style="text-align:${align[k]}"` : ""}>${inline(c)}</${tag}>`;
        let t = `<div class="mdtable"><table><thead><tr>${head.map((c, k) => td(c, k, "th")).join("")}</tr></thead><tbody>`;
        i += 2;
        while (i < lines.length && lines[i].includes("|") && lines[i].trim()) {
          const r = cells(lines[i++]);
          t += `<tr>${head.map((_, k) => td(r[k] || "", k, "td")).join("")}</tr>`;
        }
        out += t + "</tbody></table></div>";
        continue;
      }
      if (listRe.test(l) && !/^\s{4,}/.test(l)) {
        flush();
        const [html, next] = list(lines, i);
        out += html;
        i = next;
        continue;
      }
      para.push(l);
      i++;
    }
    flush();
    return out;
  }

  window.mdRender = render;
})();
