//! F10.5 · La página del teléfono: una sola, dentro del binario, sin nada de fuera.
//!
//! Prueba del Portador (2026-11-18): «toca actualizar para ver la actualización» y «la página es súper plana».
//! - **En vivo de verdad:** los eventos llegan por `EventSource` (reconecta solo y manda `Last-Event-ID`); al
//!   volver a la pestaña o desbloquear el teléfono se reconecta en el acto (iOS deja el flujo colgado SIN error
//!   cuando la pantalla se apaga) y un vigía lo renueva si pasa un rato sin noticias. La lista de chats se
//!   refresca sola.
//! - **Sin `localStorage`:** el token es una cookie HttpOnly que pone el servidor al emparejar; el JS no lo ve.
//! - **Diseño (2026-11-18, «más limpia, parecida a la app de Claude en el celular»):** cabecera simple, lista
//!   de chats en filas con línea fina, en el chat la burbuja del usuario discreta (gris) y la respuesta como
//!   texto corrido, los pasos como líneas pequeñas y grises, la caja de escribir abajo y redondeada. Superficies
//!   neutras, sin degradados ni brillos; un solo acento. El permiso, una tarjeta neutra como en la Mac: el
//!   riesgo es una etiqueta pequeña con un punto de color, nunca la tarjeta teñida.

// juez-idioma: datos, bloque (la página lleva sus textos en los dos idiomas dentro de su diccionario `T`)
pub const PAGINA: &str = r##"<!doctype html>
<html><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1,viewport-fit=cover">
<meta name="theme-color" content="#151517" media="(prefers-color-scheme: dark)">
<meta name="theme-color" content="#ffffff" media="(prefers-color-scheme: light)">
<meta name="apple-mobile-web-app-capable" content="yes">
<meta name="apple-mobile-web-app-status-bar-style" content="default">
<title>GOD OF THE VOID</title>
<style>
:root{--bg:#151517;--sf:#1e1e21;--sf2:#26262a;--hv:#2c2c31;--ln:#2e2e33;--ln2:#3a3a40;--t:#ececef;--mu:#a1a1aa;--fa:#71717a;
--ac:#8a7df3;--ac2:#6e5ff0;--acs:rgba(138,125,243,.18);--ok:#5fc48c;--bad:#ef7682;--warn:#e3b26b;color-scheme:dark}
@media (prefers-color-scheme: light){:root{--bg:#fff;--sf:#fff;--sf2:#f4f4f5;--hv:#ececee;--ln:#e7e7ea;--ln2:#d6d6db;--t:#18181b;--mu:#5f5f68;--fa:#8e8e96;
--ac:#5b4be0;--ac2:#4a39d4;--acs:rgba(91,75,224,.14);--ok:#1f9a5a;--bad:#d23c4d;--warn:#b97a16;color-scheme:light}}
*{box-sizing:border-box;-webkit-tap-highlight-color:transparent}
html,body{margin:0;background:var(--bg);color:var(--t)}
body{font:16px/1.55 -apple-system,BlinkMacSystemFont,"SF Pro Text","Helvetica Neue",system-ui,sans-serif;-webkit-font-smoothing:antialiased;min-height:100vh;min-height:100dvh}
button{font:inherit;color:inherit;cursor:pointer}
code,pre,.mono{font-family:ui-monospace,"SF Mono",Menlo,monospace}
/* cabecera: simple, del color del fondo, con una línea fina */
.top{position:sticky;top:0;z-index:5;display:flex;align-items:center;gap:8px;min-height:52px;padding:calc(env(safe-area-inset-top) + 6px) 12px 6px;background:var(--bg);border-bottom:1px solid var(--ln)}
.nom{flex:1;min-width:0;text-align:center}
.nom b{display:block;font-size:16px;font-weight:600;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}
.nom span{display:block;font-size:12px;color:var(--mu);white-space:nowrap;overflow:hidden;text-overflow:ellipsis}
.atras,.hueco{flex:none;display:grid;place-items:center;width:36px;height:36px;border:0;border-radius:10px;background:transparent;color:var(--t)}
.atras:active{background:var(--hv)}
.vivo{width:7px;height:7px;border-radius:50%;background:var(--fa);flex:none;margin:0 14px 0 15px}
.vivo.on{background:var(--ok)}
.vivo.re{background:var(--warn);animation:lat 1.2s ease-in-out infinite}
@keyframes lat{50%{opacity:.35}}
main{max-width:720px;margin:0 auto;padding:0 16px calc(env(safe-area-inset-bottom) + 24px)}
/* lista de chats */
.tit{margin:20px 4px 2px;font-size:28px;font-weight:700;letter-spacing:-.02em}
.sec{margin:0 4px 12px;font-size:14px;color:var(--mu)}
.lista{border-top:1px solid var(--ln)}
.chat{display:block;width:100%;text-align:left;border:0;border-bottom:1px solid var(--ln);background:transparent;padding:14px 4px}
.chat:active{background:var(--sf2)}
.chat .f{display:flex;align-items:baseline;gap:12px}.chat .ti{flex:1;min-width:0;font-weight:500;font-size:16px;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}
.chat .ru{margin-top:2px;font-size:13px;color:var(--fa);white-space:nowrap;overflow:hidden;text-overflow:ellipsis}
.chat .pend{display:flex;align-items:center;gap:6px;margin-top:6px;font-size:13px;color:var(--t)}
.chat .pend::before{content:"";width:6px;height:6px;border-radius:50%;background:var(--warn)}
.est{font-size:13px;color:var(--fa);white-space:nowrap}
.est.trabajando{color:var(--mu)}.est.esperando_permiso{color:var(--t)}
.vacio{text-align:center;padding:64px 16px;color:var(--mu);font-size:15px}
.vacio b{display:block;color:var(--t);font-size:17px;font-weight:600;margin-bottom:4px}
.enl{display:block;border:0;background:transparent;color:var(--mu);font-size:14px;padding:20px 0;width:100%}
/* emparejar */
.par{padding:40px 4px 0}
.par h2{margin:0 0 8px;font-size:26px;font-weight:700;letter-spacing:-.02em}.par p{margin:0 0 28px;color:var(--mu);font-size:15px}
label{display:block;font-size:13px;font-weight:500;color:var(--mu);margin:0 0 6px}
input{font:inherit;width:100%;height:48px;padding:0 14px;border-radius:12px;border:1px solid var(--ln2);background:var(--sf);color:var(--t);outline:none;margin:0 0 16px}
input:focus{border-color:var(--ac)}
.cod{font:600 22px/1 ui-monospace,"SF Mono",Menlo,monospace;letter-spacing:.3em;text-align:center}
.prim{width:100%;height:48px;border:0;border-radius:12px;background:var(--ac2);color:#fff;font-weight:600;margin-top:8px}
.prim:active{opacity:.85}.prim:disabled{opacity:.5}
.err{color:var(--bad);font-size:14px;min-height:20px;margin:12px 0 0}
/* conversación: la burbuja del usuario discreta, la respuesta como texto corrido, los pasos en gris */
#log{display:flex;flex-direction:column;gap:14px;padding:20px 0 120px}
.yo{align-self:flex-end;max-width:85%;padding:10px 14px;border-radius:18px;background:var(--sf2);color:var(--t);white-space:pre-wrap;word-break:break-word}
.yo small{display:block;font-size:12px;color:var(--fa);margin-bottom:2px}
.yo.tel{opacity:.8}
.el{white-space:pre-wrap;word-break:break-word}
.paso{display:flex;gap:8px;align-items:baseline;font-size:13px;line-height:1.45;color:var(--fa);margin:-6px 0}
.paso::before{content:"";flex:none;width:4px;height:4px;border-radius:50%;background:currentColor;transform:translateY(-2px)}
.escribe{display:flex;gap:4px;padding:4px 0}.escribe i{width:6px;height:6px;border-radius:50%;background:var(--fa);animation:lat 1s infinite}
.escribe i:nth-child(2){animation-delay:.15s}.escribe i:nth-child(3){animation-delay:.3s}
/* el permiso: tarjeta neutra, como en la Mac */
.perm{border:1px solid var(--ln);background:var(--sf);border-radius:14px;padding:16px}
.perm .cab{display:flex;align-items:flex-start;gap:10px}
.perm .ac{flex:1;min-width:0;font-weight:600;font-size:16px;line-height:1.35}
.perm .ojo{font-size:13px;color:var(--mu);font-weight:400;margin-top:2px}
.perm .ri{flex:none;display:inline-flex;align-items:center;gap:6px;height:22px;padding:0 8px;border:1px solid var(--ln);border-radius:6px;font-size:12px;color:var(--mu);white-space:nowrap}
.perm .ri::before{content:"";width:6px;height:6px;border-radius:50%;background:var(--warn)}
.perm[data-r="alto"] .ri::before,.perm[data-r="alto"] .mc::before{background:var(--bad)}.perm[data-r="bajo"] .ri::before{background:var(--ok)}
.perm .mc{display:flex;align-items:baseline;gap:8px;margin-top:10px;font-size:14px;font-weight:500}
.perm .mc::before{content:"";flex:none;width:6px;height:6px;border-radius:50%;background:var(--warn);transform:translateY(-2px)}
.perm pre{margin:12px 0 0;padding:10px 12px;border-radius:8px;background:var(--sf2);border:1px solid var(--ln);font-size:13px;line-height:1.55;white-space:pre-wrap;word-break:break-word;max-height:40vh;overflow:auto}
.perm pre .pr{color:var(--fa);margin-right:8px;user-select:none}
.perm ul{list-style:none;margin:12px 0 0;padding:0;border-radius:8px;background:var(--sf2);border:1px solid var(--ln);overflow:hidden}
.perm li{padding:8px 12px;border-top:1px solid var(--ln);font-size:13px;color:var(--mu)}.perm li:first-child{border-top:0}
.perm li code{display:block;word-break:break-word;white-space:pre-wrap;font-size:12.5px;color:var(--t)}
.perm .nt{margin-top:10px;font-size:14px;color:var(--mu)}
.perm .bt{display:grid;grid-template-columns:1fr 1fr;gap:8px;margin-top:16px}
.perm .bt button{height:44px;border-radius:10px;font-weight:600;font-size:15px;border:1px solid var(--ln2);background:var(--sf)}
.perm .bt .si{border-color:transparent;background:var(--ac2);color:#fff}
.perm .bt button:disabled{opacity:.5}
.perm .ya{font-size:14px;color:var(--mu)}
/* escribir: abajo, una caja redondeada con el botón dentro */
.pie{position:fixed;left:0;right:0;bottom:0;z-index:5;padding:8px 12px calc(env(safe-area-inset-bottom) + 10px);background:var(--bg)}
.pie .in{max-width:720px;margin:0 auto;display:flex;gap:8px;align-items:center;padding:4px 4px 4px 16px;border:1px solid var(--ln2);border-radius:24px;background:var(--sf)}
.pie input{margin:0;height:40px;padding:0;border:0;border-radius:0;background:transparent}
.env{flex:none;width:36px;height:36px;border-radius:50%;border:0;background:var(--ac2);color:#fff;display:grid;place-items:center}
.env:disabled{background:var(--sf2);color:var(--fa)}
.aviso{max-width:720px;margin:0 auto;font-size:13px;color:var(--mu);text-align:center}
.aviso:not(:empty){margin-bottom:6px}
</style></head><body>
<header class="top" id="top"></header>
<main id="v"></main>
<script>
const T={es:{par:"Empareja este teléfono",parT:"Escanea el código QR de tu Mac o escribe el código que aparece en Configuración → API → Teléfono.",cod:"Código de 8 cifras",nom:"Nombre de este teléfono",ir:"Emparejar",mal:"Código incorrecto o vencido. Pide uno nuevo en la Mac.",
titulo:"Chats",chats:"Abiertos en tu Mac",nada:"No hay chats abiertos",nadaT:"Abre un proyecto en GOD OF THE VOID y aparecerá aquí.",perm:"Pide permiso",si:"Aprobar",no:"Negar",msg:"Escribe al chat…",
enc:"En cola: el chat lo atiende como su próxima orden.",libre:"Libre",trabajando:"Trabajando…",esperando_permiso:"Esperando permiso",salir:"Olvidar este teléfono",perdidos:"Se perdieron mensajes antiguos (eran demasiados).",
ya:"Ese permiso ya se contestó.",resuelto:{si:"Aprobado",no:"Negado"},desde:{ventana:"desde la Mac",telefono:"desde este teléfono"},pend:n=>n==1?"1 permiso esperando tu respuesta":n+" permisos esperando tu respuesta",
tu:"Tú",remoto:"Control remoto",riesgo:{alto:"Riesgo alto",medio:"Riesgo medio",bajo:"Riesgo bajo"},en:"en vivo",reco:"reconectando…",atras:"Volver",enviar:"Enviar",
t:{bash:"Ejecutar un comando",write_file:"Crear o reemplazar un archivo",edit:"Editar un archivo",multi_edit:"Editar un archivo",web_fetch:"Leer una página web",web_search:"Buscar en la web",net:"Conectarse a la red",ssh:"Ejecutar por SSH",package:"Instalar un paquete",github:"Actuar en GitHub",desktop:"Controlar otra app",read_file:"Leer un archivo"},otra:"Permitir una operación",lote:"Operaciones de este paso"},
en:{par:"Pair this phone",parT:"Scan the QR code on your Mac or type the code shown in Settings → API → Phone.",cod:"8-digit code",nom:"Name of this phone",ir:"Pair",mal:"Wrong or expired code. Ask the Mac for a new one.",
titulo:"Chats",chats:"Open on your Mac",nada:"No open chats",nadaT:"Open a project in GOD OF THE VOID and it will show up here.",perm:"Asks for permission",si:"Approve",no:"Deny",msg:"Write to the chat…",
enc:"Queued: the chat handles it as its next order.",libre:"Idle",trabajando:"Working…",esperando_permiso:"Waiting for permission",salir:"Forget this phone",perdidos:"Old messages were lost (there were too many).",
ya:"That permission was already answered.",resuelto:{si:"Approved",no:"Denied"},desde:{ventana:"on the Mac",telefono:"from this phone"},pend:n=>n==1?"1 permission waiting for you":n+" permissions waiting for you",
tu:"You",remoto:"Remote control",riesgo:{alto:"High risk",medio:"Medium risk",bajo:"Low risk"},en:"live",reco:"reconnecting…",atras:"Back",enviar:"Send",
t:{bash:"Run a command",write_file:"Create or replace a file",edit:"Edit a file",multi_edit:"Edit a file",web_fetch:"Read a web page",web_search:"Search the web",net:"Connect to the network",ssh:"Run over SSH",package:"Install a package",github:"Act on GitHub",desktop:"Control another app",read_file:"Read a file"},otra:"Allow an operation",lote:"Operations in this step"}};
const L=(navigator.language||"es").toLowerCase().startsWith("es")?T.es:T.en;
const $=s=>document.querySelector(s),V=$("#v"),TOP=$("#top");
const esc=s=>String(s??"").replace(/[&<>"']/g,c=>({"&":"&amp;","<":"&lt;",">":"&gt;",'"':"&quot;","'":"&#39;"}[c]));
let vista=null,fuente=null,tLista=null,tVigia=null,reconectar=null;
function parar(){reconectar=null;if(fuente){fuente.close();fuente=null}clearInterval(tLista);clearInterval(tVigia);tLista=tVigia=null;document.querySelector(".pie")?.remove()}
async function api(m,u,b){const r=await fetch(u,{method:m,credentials:"same-origin",headers:b?{"Content-Type":"application/json"}:{},body:b?JSON.stringify(b):undefined});if(r.status==401){emparejar();throw new Error("401")}return r}
// la cabecera: volver (en el chat), el título (y el estado del chat debajo) y el punto de «en vivo»
function cabeza(titulo,atras,estado){TOP.innerHTML=(atras?`<button class="atras" id="at" aria-label="${esc(L.atras)}"><svg width="22" height="22" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="m15 18-6-6 6-6"/></svg></button>`:`<span class="hueco"></span>`)+
`<div class="nom"><b>${esc(titulo)}</b>${estado!==undefined?`<span class="est" id="es"></span>`:""}</div><span class="vivo" id="vv"></span>`;if(atras)$("#at").onclick=lista}
function vivo(c){const v=$("#vv");if(v){v.className="vivo "+c;v.title=c=="on"?L.en:c=="re"?L.reco:""}}
function emparejar(){parar();vista="par";cabeza("GOD OF THE VOID",false);
V.innerHTML=`<div class="par"><h2>${L.par}</h2><p>${L.parT}</p><label for="c">${L.cod}</label><input id="c" class="cod" inputmode="numeric" autocomplete="one-time-code" maxlength="8"><label for="n">${L.nom}</label><input id="n" autocomplete="off"><button class="prim" id="b">${L.ir}</button><p class="err" id="e"></p></div>`;
$("#b").onclick=async()=>{$("#b").disabled=true;const r=await fetch("/gotv/emparejar",{method:"POST",credentials:"same-origin",headers:{"Content-Type":"application/json"},body:JSON.stringify({codigo:$("#c").value,nombre:$("#n").value})});$("#b").disabled=false;if(!r.ok){$("#e").textContent=L.mal;return}lista()};
const h=new URLSearchParams(location.hash.slice(1)).get("c");if(h){history.replaceState(null,"",location.pathname);$("#c").value=h;$("#n").value=/iPhone/.test(navigator.userAgent)?"iPhone":/Android/.test(navigator.userAgent)?"Android":"";$("#b").click()}}
async function lista(){parar();vista="lista";cabeza(L.remoto,false);await pintarLista();tLista=setInterval(()=>{if(!document.hidden)pintarLista()},3000)}
async function pintarLista(){let d;try{d=await (await api("GET","/gotv/sesiones")).json();vivo("on")}catch(e){if(e.message!="401")vivo("re");return}if(vista!="lista")return;
V.innerHTML=`<div class="tit">${L.titulo}</div>`+(d.sesiones.length?`<div class="sec">${L.chats}</div><div class="lista">`+d.sesiones.map(s=>`<button class="chat" data-id="${esc(s.id)}"><div class="f"><span class="ti">${esc(s.titulo)}</span><span class="est ${esc(s.estado)}">${esc(L[s.estado]||s.estado)}</span></div><div class="ru">${esc(s.ruta||s.proyecto)}</div>${s.permisos&&s.permisos.length?`<div class="pend">${esc(L.pend(s.permisos.length))}</div>`:""}</button>`).join("")+`</div>`
:`<div class="vacio"><b>${L.nada}</b>${L.nadaT}</div>`)+`<button class="enl" id="x">${L.salir}</button>`;
document.querySelectorAll("[data-id]").forEach(c=>c.onclick=()=>chat(c.dataset.id,c.querySelector(".ti").textContent));
$("#x").onclick=async()=>{await fetch("/gotv/salir",{method:"POST",credentials:"same-origin"});emparejar()}}
function chat(id,titulo){parar();vista="chat";cabeza(titulo,true,"");V.innerHTML=`<div id="log"></div>`;
const pie=document.createElement("div");pie.className="pie";pie.innerHTML=`<div class="aviso" id="q"></div><div class="in"><input id="m" placeholder="${esc(L.msg)}" enterkeyhint="send"><button class="env" id="s" aria-label="${esc(L.enviar)}" disabled><svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round"><path d="M12 19V5M5 12l7-7 7 7"/></svg></button></div>`;document.body.appendChild(pie);
const G=$("#log");let ultimo=0,texto=null,escribe=null,visto=Date.now();
const abajo=()=>innerHeight+scrollY>=document.body.scrollHeight-160;
const poner=(el)=>{const ir=abajo();if(escribe&&escribe.parentNode)G.insertBefore(el,escribe);else G.appendChild(el);if(ir)scrollTo(0,document.body.scrollHeight)};
const estado=(e)=>{const c=$("#es");if(c){c.className="est "+e;c.textContent=L[e]||e}if(e=="trabajando"){if(!escribe){escribe=document.createElement("div");escribe.className="escribe";escribe.innerHTML="<i></i><i></i><i></i>";G.appendChild(escribe)}}else if(escribe){escribe.remove();escribe=null}};
const m=$("#m"),s=$("#s");m.oninput=()=>s.disabled=!m.value.trim();
const mandar=async()=>{const t=m.value.trim();if(!t)return;s.disabled=true;const r=await api("POST",`/gotv/sesiones/${encodeURIComponent(id)}/mensaje`,{texto:t});if(r.ok){m.value="";$("#q").textContent=L.enc;setTimeout(()=>{const q=$("#q");if(q)q.textContent=""},4000)}};
s.onclick=mandar;m.onkeydown=e=>{if(e.key=="Enter"){e.preventDefault();mandar()}};
function permiso(d){const p=d.partes||{riesgo:"medio",detalle:d.descripcion};const c=document.createElement("div");c.className="perm";c.id="p-"+d.id_permiso;c.dataset.r=p.riesgo||"medio";
const titulo=x=>L.t[x.herramienta]||x.herramienta||x.marca||L.otra;
let cuerpo="";if(Array.isArray(p.lote)){cuerpo=`<ul>${p.lote.map(x=>`<li>${esc(titulo(x))}<code>${esc(x.detalle||x.texto)}</code></li>`).join("")}</ul>`}
else cuerpo=(p.marca?`<div class="mc">${esc(p.marca)}</div>`:"")+`<pre>${p.herramienta=="bash"?'<span class="pr">$</span>':""}${esc(p.detalle||d.descripcion)}</pre>`+(p.nota?`<div class="nt">${esc(p.nota)}</div>`:"");
c.innerHTML=`<div class="cab"><div class="ac">${esc(Array.isArray(p.lote)?(p.titulo||L.lote):titulo(p))}<div class="ojo">${L.perm}</div></div><span class="ri">${esc(L.riesgo[p.riesgo]||L.riesgo.medio)}</span></div>${cuerpo}<div class="bt"><button class="no">${L.no}</button><button class="si">${L.si}</button></div>`;
const r=async si=>{c.querySelectorAll(".bt button").forEach(b=>b.disabled=true);const x=await api("POST",`/gotv/sesiones/${encodeURIComponent(id)}/permiso`,{id_permiso:d.id_permiso,decision:si?"si":"no"});if(x.status==409)c.innerHTML=`<div class="ya">${L.ya}</div>`};
c.querySelector(".si").onclick=()=>r(true);c.querySelector(".no").onclick=()=>r(false);texto=null;poner(c);if(!document.hidden&&navigator.vibrate)navigator.vibrate(30)}
function paso(t){const a=document.createElement("div");a.className="paso";a.textContent=t;texto=null;poner(a)}
const manejar={texto:d=>{let t=d.texto;if(!texto){t=t.replace(/^\s+/,"");if(!t)return;texto=document.createElement("div");texto.className="el";poner(texto)}const ir=abajo();texto.appendChild(document.createTextNode(t));if(ir)scrollTo(0,document.body.scrollHeight)},
orden:d=>{const a=document.createElement("div");a.className="yo";a.appendChild(document.createTextNode(d.texto));texto=null;poner(a)},
mensaje:d=>{const a=document.createElement("div");a.className="yo tel";a.innerHTML=`<small>${L.desde.telefono}</small>`;a.appendChild(document.createTextNode(d.texto));texto=null;poner(a)},
actividad:d=>paso(d.detalle),permiso:permiso,estado:d=>estado(d.estado),perdidos:()=>paso(L.perdidos),
permiso_resuelto:d=>{const c=$("#p-"+d.id_permiso);const t=(d.aprobado?L.resuelto.si:L.resuelto.no)+" "+(L.desde[d.desde]||d.desde);if(c){c.innerHTML=`<div class="ya">${esc(t)}</div>`}else paso(t)}};
function conectar(){if(fuente)fuente.close();vivo("re");fuente=new EventSource(`/gotv/sesiones/${encodeURIComponent(id)}/eventos`+(ultimo?`?desde=${ultimo}`:""));
fuente.onopen=()=>{vivo("on");visto=Date.now()};
for(const tipo of Object.keys(manejar))fuente.addEventListener(tipo,ev=>{visto=Date.now();vivo("on");const n=+ev.lastEventId;if(n&&n<=ultimo)return;if(n)ultimo=n;try{manejar[tipo](JSON.parse(ev.data))}catch(e){}});
fuente.onerror=async()=>{vivo("re");if(fuente&&fuente.readyState===2){fuente=null;try{const r=await fetch("/gotv/sesiones",{credentials:"same-origin"});if(r.status==401)return emparejar();const d=await r.json();if(!d.sesiones.some(x=>x.id==id))return lista()}catch(e){}setTimeout(()=>{if(vista=="chat")conectar()},2000)}}}
conectar();reconectar=()=>{visto=Date.now();conectar()};
// iOS deja el flujo colgado SIN error con la pantalla apagada: al volver, y si pasa un rato sin noticias, se renueva
tVigia=setInterval(()=>{if(vista=="chat"&&!document.hidden&&Date.now()-visto>45000){visto=Date.now();conectar()}},10000)}
document.addEventListener("visibilitychange",()=>{if(document.hidden)return;if(vista=="chat"&&reconectar)reconectar();else if(vista=="lista")pintarLista()});
addEventListener("hashchange",()=>{if(location.hash.includes("c="))emparejar()});
addEventListener("pageshow",e=>{if(e.persisted&&vista=="chat"&&reconectar)reconectar()});
(async()=>{if(location.hash.includes("c="))return emparejar();const r=await fetch("/gotv/sesiones",{credentials:"same-origin"});r.status==401?emparejar():lista()})();
</script></body></html>"##;

#[cfg(test)]
mod tests {
    use super::PAGINA;

    /// Diseño (2026-11-18): la página se repintó entera, pero su conducta no se toca. Lo que la hace «en vivo»
    /// y segura sigue ahí: EventSource con reconexión al volver y vigía de 45 s, dedupe por `lastEventId`,
    /// cookie (sin `localStorage`), emparejar por `#c=` y `hashchange`, aprobar/negar y la caja de escribir.
    #[test]
    fn el_rediseno_conserva_la_conducta() {
        for imprescindible in [
            "new EventSource(",
            "visibilitychange",
            "pageshow",
            "45000",
            "ev.lastEventId",
            "n<=ultimo",
            "credentials:\"same-origin\"",
            "location.hash",
            "addEventListener(\"hashchange\"",
            "/gotv/emparejar",
            "/permiso`,{id_permiso:d.id_permiso,decision:si?\"si\":\"no\"}",
            "/mensaje`,{texto:t}",
            "enterkeyhint=\"send\"",
        ] {
            assert!(PAGINA.contains(imprescindible), "la página perdió: {imprescindible}");
        }
        assert!(!PAGINA.contains("localStorage") && !PAGINA.contains("sessionStorage"), "el token no se guarda en el navegador");
    }

    /// Y se ve como se pidió: sin degradados ni brillos, sin la tarjeta del permiso teñida por el riesgo (el
    /// riesgo es un punto de color en una etiqueta), sin el escudo grande ni mayúsculas gritonas.
    #[test]
    fn la_pagina_es_limpia_y_neutra() {
        let css = &PAGINA[PAGINA.find("<style>").unwrap()..PAGINA.find("</style>").unwrap()];
        for prohibido in ["gradient", "box-shadow", "backdrop-filter", "text-transform:uppercase", "border-top:3px"] {
            assert!(!css.contains(prohibido), "estilo de antes: {prohibido}");
        }
        assert!(!PAGINA.contains("ESCUDO") && !PAGINA.contains("RIESGO ALTO"));
        assert!(css.contains(".perm .ri::before"), "el riesgo, como punto de color en su etiqueta");
    }
}
