// F16 · el único enganche en index.html: carga la hoja de estilos y las piezas de escritorio, en orden, durante
// el análisis de la página (document.write: como mock.js). Cada pieza se registra sola en F8 (gotvPanel,
// gotvAjustes) o escucha sus eventos: cola, revisión, archivo, terminal y ajustes. mock-escritorio.js sólo
// actúa fuera de Tauri.
(() => {
  const css = document.createElement("link");
  css.rel = "stylesheet";
  css.href = "escritorio.css";
  document.head.appendChild(css);
  const piezas = ["mock-escritorio.js", "escritorio-i18n.js", "escritorio-cola.js", "escritorio-revision.js", "escritorio-archivo.js", "escritorio-terminal.js", "escritorio-ajustes.js"];
  document.write(piezas.map((p) => `<script src="${p}"><\/script>`).join(""));
})();
