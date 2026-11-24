import { useMemo, useState } from "react";
import { Aplicacion, Avisos, Barras, Boton, Campo, Dialogo, Estadistica, InsigniaEstado, Lineas, Panel, Pestanas, Tabla, useAviso, type Columna } from "./gotv/gotv";
import { borrar, crear, HISTORIA, porCiudad, resumen, SEMILLA, validar, type Borrador, type Cliente, type Errores } from "./datos";

const TONO = { activo: "exito", pendiente: "aviso", inactivo: "peligro" } as const;
const VACIO: Borrador = { nombre: "", correo: "", ciudad: "" };

function Clientes() {
  const avisar = useAviso();
  const [lista, setLista] = useState<Cliente[]>(SEMILLA);
  const [filtro, setFiltro] = useState("todos");
  const [abierto, setAbierto] = useState(false);
  const [borrador, setBorrador] = useState<Borrador>(VACIO);
  const [errores, setErrores] = useState<Errores>({});
  const r = useMemo(() => resumen(lista), [lista]);
  const filas = filtro === "todos" ? lista : lista.filter((c) => c.estado === filtro);

  const columnas: Columna<Cliente>[] = [
    { clave: "nombre", titulo: "Nombre" },
    { clave: "correo", titulo: "Correo" },
    { clave: "ciudad", titulo: "Ciudad" },
    { clave: "estado", titulo: "Estado", render: (c) => <InsigniaEstado tono={TONO[c.estado]}>{c.estado}</InsigniaEstado> },
    { clave: "compras", titulo: "Compras", num: true },
    { clave: "id", titulo: "Acciones", ordenable: false, render: (c) => (
      <Boton variante="peligro" chico aria-label={`Borrar a ${c.nombre}`} onClick={() => { setLista(borrar(c.id, lista)); avisar(`${c.nombre} se borró.`, "info"); }}>Borrar</Boton>
    ) },
  ];

  const guardar = () => {
    const e = validar(borrador, lista);
    setErrores(e);
    if (Object.keys(e).length) return;
    setLista(crear(borrador, lista));
    setAbierto(false);
    setBorrador(VACIO);
    avisar(`${borrador.nombre.trim()} quedó guardado.`);
  };

  return (
    <>
      <header className="app__cabeza">
        <div><h1>Clientes</h1><p>Quién te compra, desde dónde y cuánto.</p></div>
        <Boton icono="plus" onClick={() => { setErrores({}); setAbierto(true); }}>Nuevo cliente</Boton>
      </header>
      <section className="estadisticas" aria-label="Resumen">
        <Estadistica etiqueta="Clientes" valor={r.total} />
        <Estadistica etiqueta="Activos" valor={r.activos} cambio={12.5} />
        <Estadistica etiqueta="Compras" valor={r.compras} cambio={-3} serie={HISTORIA.nuevos.map((v, i) => v + HISTORIA.recurrentes[i])} />
        <Estadistica etiqueta="Promedio por cliente" valor={r.promedio.toLocaleString("es", { maximumFractionDigits: 1 })} />
      </section>
      <div className="paneles">
        <Panel titulo="Compras por ciudad">
          <Barras titulo="Compras por ciudad" datos={porCiudad(lista)} parte />
        </Panel>
        <Panel titulo="Compras por mes">
          <Lineas titulo="Compras por mes" eje="Mes" etiquetas={HISTORIA.meses} series={[
            { nombre: "Recurrentes", valores: HISTORIA.recurrentes }, { nombre: "Clientes nuevos", valores: HISTORIA.nuevos },
          ]} />
        </Panel>
      </div>
      <Pestanas activa={filtro} alCambiar={setFiltro} pestanas={[
        { id: "todos", titulo: "Todos" }, { id: "activo", titulo: "Activos" }, { id: "pendiente", titulo: "Pendientes" }, { id: "inactivo", titulo: "Inactivos" },
      ]} />
      <Tabla titulo="Lista de clientes" columnas={columnas} filas={filas} buscarEn={["nombre", "correo", "ciudad"]} />
      <Dialogo abierto={abierto} titulo="Nuevo cliente" alCerrar={() => setAbierto(false)}
        pie={<><Boton variante="secundario" onClick={() => setAbierto(false)}>Cancelar</Boton><Boton onClick={guardar}>Guardar</Boton></>}>
        <Campo etiqueta="Nombre" autoComplete="name" value={borrador.nombre} error={errores.nombre} onChange={(e) => setBorrador({ ...borrador, nombre: e.target.value })} />
        <Campo etiqueta="Correo" type="email" autoComplete="email" value={borrador.correo} error={errores.correo} onChange={(e) => setBorrador({ ...borrador, correo: e.target.value })} />
        <Campo etiqueta="Ciudad" value={borrador.ciudad} error={errores.ciudad} onChange={(e) => setBorrador({ ...borrador, ciudad: e.target.value })} />
      </Dialogo>
    </>
  );
}

export default function App() {
  return (
    <Avisos>
      <Aplicacion marca={__MARCA__} actual="clientes" enlaces={[
        { id: "inicio", texto: "Inicio", icono: "house" },
        { id: "clientes", texto: "Clientes", icono: "users" },
        { id: "ventas", texto: "Ventas", icono: "chart-column" },
        { id: "ajustes", texto: "Ajustes", icono: "settings" },
      ]}>
        <Clientes />
      </Aplicacion>
    </Avisos>
  );
}
