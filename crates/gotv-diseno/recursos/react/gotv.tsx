// GOD OF THE VOID · biblioteca de componentes para React (Olas D5 y D3). Las MISMAS clases que la
// hoja `estilos.css` (tema calculado + gotv.css): una fuente, dos puertas. Sin dependencias
// además de React. Accesible por construcción: etiquetas, foco, roles y teclado. Licencia MIT.
import {
  type ReactNode, type InputHTMLAttributes, type ButtonHTMLAttributes, type TextareaHTMLAttributes,
  type CSSProperties, type KeyboardEvent, type PointerEvent,
  createContext, useCallback, useContext, useEffect, useId, useMemo, useRef, useState,
} from "react";

// ── iconos: los mismos SVG de la página (Lucide, ISC), por nombre ──────────────
import { ICONOS } from "./iconos";

export function Icono({ nombre, className }: { nombre: keyof typeof ICONOS | string; className?: string }) {
  const svg = (ICONOS as Record<string, string>)[nombre];
  if (!svg) return null;
  return <span className={className} style={{ display: "inline-grid" }} aria-hidden="true" dangerouslySetInnerHTML={{ __html: svg }} />;
}

// ── botón ───────────────────────────────────────────────────────────────────
type Variante = "primario" | "secundario" | "peligro";
export function Boton({ variante = "primario", chico, icono, cargando, children, ...resto }:
  ButtonHTMLAttributes<HTMLButtonElement> & { variante?: Variante; chico?: boolean; icono?: string; cargando?: boolean }) {
  return (
    <button {...resto} disabled={resto.disabled || cargando} aria-busy={cargando || undefined}
      className={`boton boton--${variante}${chico ? " boton--chico" : ""}${resto.className ? " " + resto.className : ""}`}>
      {cargando ? <Icono nombre="loader-circle" className="girando" /> : null}
      {children}
      {icono && !cargando ? <Icono nombre={icono} /> : null}
    </button>
  );
}

// ── campo con etiqueta, ayuda y error ───────────────────────────────────────
export function Campo({ etiqueta, ayuda, error, ...input }: InputHTMLAttributes<HTMLInputElement> & { etiqueta: string; ayuda?: string; error?: string }) {
  const id = useId();
  const desc = [ayuda && `${id}-ayuda`, error && `${id}-error`].filter(Boolean).join(" ") || undefined;
  return (
    <div className="campo">
      <label htmlFor={id}>{etiqueta}</label>
      <input id={id} {...input} aria-invalid={error ? true : undefined} aria-describedby={desc} />
      {ayuda ? <small id={`${id}-ayuda`}>{ayuda}</small> : null}
      {error ? <p className="campo__error" id={`${id}-error`} role="alert">{error}</p> : null}
    </div>
  );
}

// ── selector con etiqueta y error (para campos de opciones) ─────────────────
export function Selector({ etiqueta, opciones, valor, alCambiar, error }:
  { etiqueta: string; opciones: string[]; valor: string; alCambiar: (v: string) => void; error?: string }) {
  const id = useId();
  return (
    <div className="campo">
      <label htmlFor={id}>{etiqueta}</label>
      <select id={id} value={valor} onChange={(e) => alCambiar(e.target.value)} aria-invalid={error ? true : undefined} aria-describedby={error ? `${id}-error` : undefined}>
        {opciones.map((o) => <option key={o} value={o}>{o}</option>)}
      </select>
      {error ? <p className="campo__error" id={`${id}-error`} role="alert">{error}</p> : null}
    </div>
  );
}

// ── texto largo, casilla e interruptor ──────────────────────────────────────
export function AreaTexto({ etiqueta, ayuda, error, ...area }: TextareaHTMLAttributes<HTMLTextAreaElement> & { etiqueta: string; ayuda?: string; error?: string }) {
  const id = useId();
  const desc = [ayuda && `${id}-ayuda`, error && `${id}-error`].filter(Boolean).join(" ") || undefined;
  return (
    <div className="campo">
      <label htmlFor={id}>{etiqueta}</label>
      <textarea id={id} {...area} aria-invalid={error ? true : undefined} aria-describedby={desc} />
      {ayuda ? <small id={`${id}-ayuda`}>{ayuda}</small> : null}
      {error ? <p className="campo__error" id={`${id}-error`} role="alert">{error}</p> : null}
    </div>
  );
}

export function Casilla({ etiqueta, ayuda, marcada, alCambiar, ...input }:
  Omit<InputHTMLAttributes<HTMLInputElement>, "type" | "checked" | "onChange"> & { etiqueta: string; ayuda?: string; marcada: boolean; alCambiar: (v: boolean) => void }) {
  return (
    <label className="casilla">
      <input {...input} type="checkbox" checked={marcada} onChange={(e) => alCambiar(e.target.checked)} />
      <span>{etiqueta}{ayuda ? <small>{ayuda}</small> : null}</span>
    </label>
  );
}

/** Para lo que se aplica AL INSTANTE (un ajuste); lo que espera al botón de guardar es una `Casilla`. */
export function Interruptor({ etiqueta, activo, alCambiar, disabled }: { etiqueta: string; activo: boolean; alCambiar: (v: boolean) => void; disabled?: boolean }) {
  return (
    <button type="button" className="interruptor" role="switch" aria-checked={activo} disabled={disabled} onClick={() => alCambiar(!activo)}>
      <span className="interruptor__riel" aria-hidden="true" />
      <span>{etiqueta}</span>
    </button>
  );
}

// ── estadística ─────────────────────────────────────────────────────────────
/** `serie`: los últimos valores de la cifra, para dibujar su tendencia al lado (decorativa: el número manda). */
export function Estadistica({ etiqueta, valor, cambio, serie }: { etiqueta: string; valor: ReactNode; cambio?: number; serie?: number[] }) {
  return (
    <div className="estadistica">
      <span>{etiqueta}</span>
      <strong>{valor}</strong>
      {serie && serie.length > 1 ? <Chispa valores={serie} /> : null}
      {cambio !== undefined ? (
        <small className={cambio >= 0 ? "sube" : "baja"}>
          {cambio >= 0 ? "▲" : "▼"} {Math.abs(cambio).toLocaleString("es", { maximumFractionDigits: 1 })} %
          <span className="solo-lector">{cambio >= 0 ? " de subida" : " de bajada"}</span>
        </small>
      ) : null}
    </div>
  );
}

export function Panel({ titulo, acciones, children }: { titulo: string; acciones?: ReactNode; children: ReactNode }) {
  return (
    <section className="panel" aria-label={titulo}>
      <div className="panel__cabeza"><h2>{titulo}</h2>{acciones}</div>
      {children}
    </section>
  );
}

export function InsigniaEstado({ tono, children }: { tono: "exito" | "aviso" | "peligro" | "info"; children: ReactNode }) {
  return <span className={`insignia-estado insignia-estado--${tono}`}>{children}</span>;
}

// ── estados: vacío, cargando, error ─────────────────────────────────────────
export function EstadoVacio({ titulo, texto, icono = "folder", accion }: { titulo: string; texto?: string; icono?: string; accion?: ReactNode }) {
  return (
    <div className="estado-vacio" role="status">
      <Icono nombre={icono} />
      <strong>{titulo}</strong>
      {texto ? <p>{texto}</p> : null}
      {accion}
    </div>
  );
}

export function Cargando({ filas = 4 }: { filas?: number }) {
  return (
    <div className="panel__cuerpo" role="status" aria-live="polite" style={{ display: "grid", gap: "var(--e-3)" }}>
      <span className="solo-lector">Cargando…</span>
      {Array.from({ length: filas }, (_, i) => <div key={i} className="esqueleto" style={{ width: `${90 - i * 12}%` }} />)}
    </div>
  );
}

export function EstadoError({ texto, reintentar }: { texto: string; reintentar?: () => void }) {
  return (
    <div className="estado-vacio" role="alert">
      <Icono nombre="triangle-alert" />
      <strong>Algo salió mal</strong>
      <p>{texto}</p>
      {reintentar ? <Boton variante="secundario" onClick={reintentar}>Reintentar</Boton> : null}
    </div>
  );
}

// ── gráficas (SVG y CSS propios, sin dependencias) ──────────────────────────
// Reglas de la casa: una sola escala por gráfica (nunca dos ejes); las marcas llevan el color de
// la serie (`--serie-1`…`--serie-6`, en ese orden, fijos con cualquier marca) y el TEXTO nunca; con
// dos series o más hay leyenda; toda gráfica deja leer sus números sin depender del color.
const num = (n: number) => n.toLocaleString("es", { maximumFractionDigits: 1 });

export function Chispa({ valores }: { valores: number[] }) {
  const lo = Math.min(...valores), hi = Math.max(...valores);
  const p = valores.map((v, i) => `${((i / (valores.length - 1)) * 100).toFixed(1)},${(26 - ((v - lo) / (hi - lo || 1)) * 24).toFixed(1)}`);
  return (
    <svg className="chispa" viewBox="0 0 100 28" preserveAspectRatio="none" aria-hidden="true">
      <polygon points={`0,28 ${p.join(" ")} 100,28`} />
      <polyline points={p.join(" ")} />
    </svg>
  );
}

export type Dato = { etiqueta: string; valor: number };

/** Cantidades (≥ 0) por categoría: una barra por fila con su valor en la punta. Con más de `max`
 *  categorías, las menores se suman en «Otras». `parte` añade el % del total. */
export function Barras({ titulo, datos, formato = num, parte, max = 8 }:
  { titulo: string; datos: Dato[]; formato?: (n: number) => string; parte?: boolean; max?: number }) {
  if (!datos.length) return <EstadoVacio titulo="Sin datos" icono="chart-column" />;
  const orden = datos.length > max ? [...datos].sort((a, b) => b.valor - a.valor) : datos;
  const filas = orden.length > max ? [...orden.slice(0, max - 1), { etiqueta: "Otras", valor: orden.slice(max - 1).reduce((s, d) => s + d.valor, 0) }] : orden;
  const tope = Math.max(...filas.map((d) => d.valor), 0) || 1;
  const total = filas.reduce((s, d) => s + Math.max(0, d.valor), 0) || 1;
  const pct = (d: Dato) => ` · ${num((Math.max(0, d.valor) / total) * 100)} %`;
  const hueco = Math.max(...filas.map((d) => (formato(d.valor) + (parte ? pct(d) : "")).length)) + 1;
  return (
    <figure className="grafica" aria-label={titulo}>
      <ul className="barras" style={{ "--hueco": `${hueco}ch` } as CSSProperties}>
        {filas.map((d) => (
          <li key={d.etiqueta} className="barras__fila">
            <span className="barras__etiqueta">{d.etiqueta}</span>
            <span className="barras__pista">
              <span className="barras__barra" style={{ "--p": Math.max(0, d.valor) / tope } as CSSProperties} />
              <span className="barras__valor">{formato(d.valor)}{parte ? <small>{pct(d)}</small> : null}</span>
            </span>
          </li>
        ))}
      </ul>
    </figure>
  );
}

/** Marcas redondas del eje: de 0 (o del mínimo, si hay negativos) al máximo, en pasos de 1, 2, 2,5 o 5 × 10ⁿ. */
function escala(min: number, max: number) {
  const lo0 = Math.min(0, min), hi0 = Math.max(0, max);
  const bruto = (hi0 - lo0) / 4 || 1;
  const p = 10 ** Math.floor(Math.log10(bruto));
  const paso = [1, 2, 2.5, 5, 10].map((m) => m * p).find((s) => s >= bruto) ?? 10 * p;
  const lo = Math.floor(lo0 / paso) * paso, hi = Math.ceil(hi0 / paso) * paso || paso;
  const marcas: number[] = [];
  for (let k = Math.round(lo / paso); k <= Math.round(hi / paso); k++) marcas.push(k * paso);
  return { lo, hi, marcas };
}

export type Serie = { nombre: string; valores: (number | null)[] };

/** Cómo cambia algo en el tiempo: una línea por serie (hasta 6; las demás quedan en la tabla).
 *  `etiquetas` son los puntos del eje (meses, días…) y cada serie trae un valor por etiqueta
 *  (`null` = sin dato: la línea se corta). Se recorre con el ratón o con las flechas. */
export function Lineas({ titulo, etiquetas, series, formato = num, eje = "", alto = 240 }:
  { titulo: string; etiquetas: string[]; series: Serie[]; formato?: (n: number) => string; eje?: string; alto?: number }) {
  const caja = useRef<HTMLDivElement>(null);
  const [ancho, setAncho] = useState(640);
  const [sel, setSel] = useState<number | null>(null);
  const [tabla, setTabla] = useState(false);
  useEffect(() => {
    const el = caja.current;
    if (!el || typeof ResizeObserver === "undefined") return;
    const ro = new ResizeObserver(([e]) => setAncho(Math.max(240, Math.round(e.contentRect.width))));
    ro.observe(el);
    return () => ro.disconnect();
  }, []);
  const n = etiquetas.length;
  const vis = series.slice(0, 6);
  const val = (s: Serie, j: number) => { const v = s.valores[j]; return typeof v === "number" && Number.isFinite(v) ? v : null; };
  const todos = vis.flatMap((s) => etiquetas.map((_, j) => val(s, j))).filter((v): v is number => v !== null);
  if (!n || !todos.length) return <EstadoVacio titulo="Sin datos" icono="chart-column" />;
  const { lo, hi, marcas } = escala(Math.min(...todos), Math.max(...todos));
  const varias = vis.length > 1;
  const fines = vis.flatMap((s, k) => {
    for (let j = n - 1; j >= 0; j--) { const v = val(s, j); if (v !== null) return [{ k, j, v }]; }
    return [];
  });
  // rótulo directo en la punta: el nombre (varias series, hasta 4) o el último valor (una sola)
  const rotulos = ancho >= 420 && vis.length <= 4;
  const izq = Math.max(...marcas.map((m) => formato(m).length)) * 7 + 14;
  const cabe = rotulos ? Math.min(16, Math.max(...fines.map((f) => (varias ? vis[f.k].nombre : formato(f.v)).length))) : 0;
  const der = rotulos ? cabe * 7 + 18 : 12;
  const arr = 12, aba = 28;
  const w = Math.max(40, ancho - izq - der), h = alto - arr - aba;
  const x = (j: number) => izq + (n === 1 ? w / 2 : (j / (n - 1)) * w);
  const y = (v: number) => arr + (1 - (v - lo) / (hi - lo)) * h;
  const trazo = (s: Serie) => {
    let d = "", pluma = false;
    for (let j = 0; j < n; j++) {
      const v = val(s, j);
      if (v === null) { pluma = false; continue; }
      d += `${pluma ? "L" : "M"}${x(j).toFixed(1)} ${y(v).toFixed(1)} `;
      pluma = true;
    }
    return d.trim();
  };
  const entera = !varias && vis[0].valores.slice(0, n).every((_, j) => val(vis[0], j) !== null) && n > 1;
  // los rótulos de las puntas no se pisan: se separan 14 px sin salirse del lienzo
  const pos = [...fines].sort((a, b) => y(a.v) - y(b.v)).map((f) => ({ ...f, yy: y(f.v) }));
  for (let q = 1; q < pos.length; q++) pos[q].yy = Math.max(pos[q].yy, pos[q - 1].yy + 14);
  for (let q = pos.length - 1; q >= 0; q--) pos[q].yy = Math.min(pos[q].yy, q === pos.length - 1 ? arr + h : pos[q + 1].yy - 14);
  const corto = (t: string) => (t.length > cabe ? t.slice(0, Math.max(1, cabe - 1)) + "…" : t);
  const cada = Math.max(1, Math.ceil(n / Math.max(1, Math.floor(w / 64))));
  const mover = (e: PointerEvent<SVGSVGElement>) => {
    const px = e.clientX - e.currentTarget.getBoundingClientRect().left;
    setSel(n === 1 ? 0 : Math.max(0, Math.min(n - 1, Math.round(((px - izq) / w) * (n - 1)))));
  };
  const tecla = (e: KeyboardEvent<HTMLDivElement>) => {
    const d = e.key === "ArrowRight" ? 1 : e.key === "ArrowLeft" ? -1 : 0;
    if (d) { e.preventDefault(); setSel((s) => Math.max(0, Math.min(n - 1, s === null ? (d > 0 ? 0 : n - 1) : s + d))); }
    else if (e.key === "Home") { e.preventDefault(); setSel(0); }
    else if (e.key === "End") { e.preventDefault(); setSel(n - 1); }
    else if (e.key === "Escape") setSel(null);
  };
  const color = (k: number) => `var(--serie-${k + 1})`;
  return (
    <figure className="grafica" aria-label={titulo}>
      {varias ? (
        <ul className="grafica__leyenda">
          {vis.map((s, k) => <li key={s.nombre}><span className="grafica__muestra" style={{ background: color(k) }} />{s.nombre}</li>)}
          {series.length > vis.length ? <li>+{series.length - vis.length} más en la tabla</li> : null}
        </ul>
      ) : null}
      <div ref={caja} className="grafica__lienzo" role="group" tabIndex={0} onKeyDown={tecla} onBlur={() => setSel(null)}
        aria-label={`${titulo}. Flechas izquierda y derecha para recorrer los puntos.`}>
        <svg height={alto} viewBox={`0 0 ${ancho} ${alto}`} aria-hidden="true" onPointerMove={mover} onPointerDown={mover} onPointerLeave={() => setSel(null)}>
          {marcas.map((m) => (
            <g key={m}>
              <line className={m === 0 ? "grafica__base" : "grafica__rejilla"} x1={izq} x2={izq + w} y1={y(m)} y2={y(m)} />
              <text className="grafica__eje" x={izq - 8} y={y(m)} dy="0.32em" textAnchor="end">{formato(m)}</text>
            </g>
          ))}
          {etiquetas.map((t, j) => ((n - 1 - j) % cada === 0 ? (
            <text key={j} className="grafica__eje" x={x(j)} y={arr + h + 18} textAnchor={n === 1 ? "middle" : j === 0 ? "start" : j === n - 1 ? "end" : "middle"}>{t}</text>
          ) : null))}
          {entera ? <path className="grafica__area" fill={color(0)} d={`${trazo(vis[0])} L${x(n - 1).toFixed(1)} ${y(Math.max(lo, 0)).toFixed(1)} L${x(0).toFixed(1)} ${y(Math.max(lo, 0)).toFixed(1)} Z`} /> : null}
          {vis.map((s, k) => <path key={s.nombre} className="grafica__linea" stroke={color(k)} d={trazo(s)} />)}
          {sel !== null ? <line className="grafica__mira" x1={x(sel)} x2={x(sel)} y1={arr} y2={arr + h} /> : null}
          {fines.map((f) => <circle key={f.k} className="grafica__punto" fill={color(f.k)} cx={x(f.j)} cy={y(f.v)} r={4} />)}
          {sel !== null ? vis.map((s, k) => { const v = val(s, sel); return v === null ? null : <circle key={k} className="grafica__punto" fill={color(k)} cx={x(sel)} cy={y(v)} r={5} />; }) : null}
          {rotulos ? pos.map((f) => (
            <text key={f.k} className="grafica__rotulo" x={x(f.j) + 10} y={f.yy} dy="0.32em">{corto(varias ? vis[f.k].nombre : formato(f.v))}</text>
          )) : null}
        </svg>
        <div className="grafica__nota" role="status" aria-live="polite"
          style={sel === null ? undefined : { left: x(sel), transform: x(sel) > ancho / 2 ? "translateX(calc(-100% - 12px))" : "translateX(12px)" }}>
          {sel === null ? null : (
            <>
              <strong>{etiquetas[sel]}</strong>
              {vis.map((s, k) => { const v = val(s, sel); return <span key={s.nombre}><i className="grafica__muestra" style={{ background: color(k) }} />{s.nombre}<b>{v === null ? "—" : formato(v)}</b></span>; })}
            </>
          )}
        </div>
      </div>
      <button type="button" className="grafica__ver" aria-expanded={tabla} onClick={() => setTabla(!tabla)}>{tabla ? "Ocultar la tabla" : "Ver como tabla"}</button>
      {tabla ? (
        <div className="tabla-marco">
          <table className="tabla">
            <caption className="solo-lector">{titulo}</caption>
            <thead><tr>{eje ? <th scope="col">{eje}</th> : <td />}{series.map((s) => <th key={s.nombre} scope="col" className="num">{s.nombre}</th>)}</tr></thead>
            <tbody>{etiquetas.map((t, j) => (
              <tr key={j}><th scope="row">{t}</th>{series.map((s) => { const v = val(s, j); return <td key={s.nombre} className="num">{v === null ? "—" : formato(v)}</td>; })}</tr>
            ))}</tbody>
          </table>
        </div>
      ) : null}
    </figure>
  );
}

// ── tabla con orden, filtro y paginación ────────────────────────────────────
export type Columna<T> = { clave: keyof T & string; titulo: string; num?: boolean; ordenable?: boolean; render?: (fila: T) => ReactNode };

export function Tabla<T extends { id: string | number }>({ titulo, columnas, filas, porPagina = 8, buscarEn, acciones, vacio }:
  { titulo: string; columnas: Columna<T>[]; filas: T[]; porPagina?: number; buscarEn?: (keyof T & string)[]; acciones?: ReactNode; vacio?: ReactNode }) {
  const [orden, setOrden] = useState<{ clave: string; asc: boolean } | null>(null);
  const [q, setQ] = useState("");
  const [pag, setPag] = useState(0);
  const visibles = useMemo(() => {
    const t = q.trim().toLowerCase();
    let v = t && buscarEn ? filas.filter((f) => buscarEn.some((k) => String(f[k] ?? "").toLowerCase().includes(t))) : filas;
    if (orden) {
      const k = orden.clave as keyof T;
      v = [...v].sort((a, b) => {
        const x = a[k], y = b[k];
        const c = typeof x === "number" && typeof y === "number" ? x - y : String(x).localeCompare(String(y), "es", { numeric: true });
        return orden.asc ? c : -c;
      });
    }
    return v;
  }, [filas, q, orden, buscarEn]);
  const paginas = Math.max(1, Math.ceil(visibles.length / porPagina));
  const p = Math.min(pag, paginas - 1);
  const trozo = visibles.slice(p * porPagina, (p + 1) * porPagina);
  return (
    <Panel titulo={titulo} acciones={
      <div className="botones">
        {buscarEn ? (
          <label className="buscador">
            <span className="solo-lector">Buscar en {titulo}</span>
            <Icono nombre="search" />
            <input type="search" placeholder="Buscar…" value={q} onChange={(e) => { setQ(e.target.value); setPag(0); }} />
          </label>
        ) : null}
        {acciones}
      </div>}>
      {visibles.length === 0 ? (vacio ?? <EstadoVacio titulo={q ? "Sin resultados" : "Aún no hay nada aquí"} texto={q ? `Nada coincide con «${q}».` : undefined} icono={q ? "search" : "folder"} />) : (
        <>
          <div className="tabla-marco">
            <table className="tabla">
              <caption className="solo-lector">{titulo}</caption>
              <thead><tr>{columnas.map((c) => {
                const activa = orden?.clave === c.clave;
                if (c.ordenable === false) return <th key={c.clave} className={c.num ? "num" : undefined}>{c.titulo}</th>;
                return (
                  <th key={c.clave} className={c.num ? "num" : undefined} aria-sort={activa ? (orden!.asc ? "ascending" : "descending") : "none"}>
                    <button type="button" onClick={() => setOrden(activa ? { clave: c.clave, asc: !orden!.asc } : { clave: c.clave, asc: true })}>
                      {c.titulo}<Icono nombre="chevron-up" />
                    </button>
                  </th>
                );
              })}</tr></thead>
              <tbody>{trozo.map((f) => (
                <tr key={f.id}>{columnas.map((c) => <td key={c.clave} className={c.num ? "num" : undefined}>{c.render ? c.render(f) : String(f[c.clave] ?? "")}</td>)}</tr>
              ))}</tbody>
            </table>
          </div>
          <nav className="paginacion" aria-label={`Páginas de ${titulo}`}>
            <span>{p * porPagina + 1}–{Math.min(visibles.length, (p + 1) * porPagina)} de {visibles.length}</span>
            <div className="botones">
              <Boton variante="secundario" chico disabled={p === 0} onClick={() => setPag(p - 1)}>Anterior</Boton>
              <Boton variante="secundario" chico disabled={p >= paginas - 1} onClick={() => setPag(p + 1)}>Siguiente</Boton>
            </div>
          </nav>
        </>
      )}
    </Panel>
  );
}

// ── diálogo (el <dialog> nativo: foco atrapado y Esc gratis) ───────────────
export function Dialogo({ abierto, titulo, alCerrar, children, pie }: { abierto: boolean; titulo: string; alCerrar: () => void; children: ReactNode; pie?: ReactNode }) {
  const ref = useRef<HTMLDialogElement>(null);
  const id = useId();
  useEffect(() => {
    const d = ref.current;
    if (!d) return;
    // respaldo para entornos sin <dialog> modal (navegadores viejos, jsdom): el atributo `open`
    if (abierto && !d.open) {
      try { d.showModal(); } catch { d.setAttribute("open", ""); }
    }
    if (!abierto && d.open) {
      try { d.close(); } catch { d.removeAttribute("open"); }
    }
  }, [abierto]);
  return (
    <dialog ref={ref} className="dialogo" aria-labelledby={id} onClose={alCerrar} onCancel={(e) => { e.preventDefault(); alCerrar(); }}>
      <div className="dialogo__cabeza">
        <h2 id={id}>{titulo}</h2>
        <button type="button" className="boton-icono" onClick={alCerrar} aria-label="Cerrar"><Icono nombre="x" /></button>
      </div>
      <div className="dialogo__cuerpo">{children}</div>
      {pie ? <div className="dialogo__pie">{pie}</div> : null}
    </dialog>
  );
}

// ── pestañas (flechas del teclado, roles ARIA) ──────────────────────────────
export function Pestanas({ pestanas, activa, alCambiar }: { pestanas: { id: string; titulo: string }[]; activa: string; alCambiar: (id: string) => void }) {
  return (
    <div className="pestanas" role="tablist" onKeyDown={(e) => {
      const i = pestanas.findIndex((p) => p.id === activa);
      const d = e.key === "ArrowRight" ? 1 : e.key === "ArrowLeft" ? -1 : 0;
      if (d) { e.preventDefault(); alCambiar(pestanas[(i + d + pestanas.length) % pestanas.length].id); }
    }}>
      {pestanas.map((p) => (
        <button key={p.id} role="tab" type="button" aria-selected={p.id === activa} tabIndex={p.id === activa ? 0 : -1} onClick={() => alCambiar(p.id)}>{p.titulo}</button>
      ))}
    </div>
  );
}

// ── avisos flotantes ────────────────────────────────────────────────────────
type Aviso = { id: number; texto: string; tono: "exito" | "peligro" | "info" };
const CtxAvisos = createContext<(texto: string, tono?: Aviso["tono"]) => void>(() => {});
export const useAviso = () => useContext(CtxAvisos);

export function Avisos({ children }: { children: ReactNode }) {
  const [lista, setLista] = useState<Aviso[]>([]);
  const avisar = useCallback((texto: string, tono: Aviso["tono"] = "exito") => {
    const id = Date.now() + Math.random();
    setLista((l) => [...l, { id, texto, tono }]);
    setTimeout(() => setLista((l) => l.filter((a) => a.id !== id)), 4000);
  }, []);
  return (
    <CtxAvisos.Provider value={avisar}>
      {children}
      <div className="avisos" role="status" aria-live="polite">
        {lista.map((a) => (
          <div key={a.id} className={`aviso-flotante aviso-flotante--${a.tono}`}>
            <Icono nombre={a.tono === "peligro" ? "circle-x" : a.tono === "info" ? "info" : "circle-check"} />
            <span>{a.texto}</span>
          </div>
        ))}
      </div>
    </CtxAvisos.Provider>
  );
}

// ── esqueleto de aplicación: barra lateral + principal ──────────────────────
export function Aplicacion({ marca, icono = "layers", enlaces, actual, children }:
  { marca: string; icono?: string; enlaces: { id: string; texto: string; icono: string }[]; actual: string; children: ReactNode }) {
  return (
    <div className="app">
      <a className="salta" href="#principal">Saltar al contenido</a>
      <aside className="app__lado" aria-label="Navegación">
        <span className="marca"><Icono nombre={icono} /><span>{marca}</span></span>
        <nav>{enlaces.map((e) => (
          <a key={e.id} className="app__enlace" href={`#${e.id}`} aria-current={e.id === actual ? "page" : undefined}><Icono nombre={e.icono} />{e.texto}</a>
        ))}</nav>
      </aside>
      <main className="app__principal" id="principal">{children}</main>
    </div>
  );
}
