import { useMemo, useState } from "react";
import { Aplicacion, Avisos, Barras, Boton, Campo, Dialogo, Estadistica, InsigniaEstado, Lineas, Panel, Pestanas, Tabla, useAviso, type Columna } from "./gotv/gotv";
import { borrar, crear, HISTORIA, porCiudad, resumen, SEMILLA, validar, type Borrador, type Cliente, type Errores } from "./datos";

const TONO = { active: "exito", pending: "aviso", inactive: "peligro" } as const;
const VACIO: Borrador = { nombre: "", correo: "", ciudad: "" };

function Clientes() {
  const avisar = useAviso();
  const [lista, setLista] = useState<Cliente[]>(SEMILLA);
  const [filtro, setFiltro] = useState("all");
  const [abierto, setAbierto] = useState(false);
  const [borrador, setBorrador] = useState<Borrador>(VACIO);
  const [errores, setErrores] = useState<Errores>({});
  const r = useMemo(() => resumen(lista), [lista]);
  const filas = filtro === "all" ? lista : lista.filter((c) => c.estado === filtro);

  const columnas: Columna<Cliente>[] = [
    { clave: "nombre", titulo: "Name" },
    { clave: "correo", titulo: "Email" },
    { clave: "ciudad", titulo: "City" },
    { clave: "estado", titulo: "Status", render: (c) => <InsigniaEstado tono={TONO[c.estado]}>{c.estado}</InsigniaEstado> },
    { clave: "compras", titulo: "Purchases", num: true },
    { clave: "id", titulo: "Actions", ordenable: false, render: (c) => (
      <Boton variante="peligro" chico aria-label={`Delete ${c.nombre}`} onClick={() => { setLista(borrar(c.id, lista)); avisar(`${c.nombre} was deleted.`, "info"); }}>Delete</Boton>
    ) },
  ];

  const guardar = () => {
    const e = validar(borrador, lista);
    setErrores(e);
    if (Object.keys(e).length) return;
    setLista(crear(borrador, lista));
    setAbierto(false);
    setBorrador(VACIO);
    avisar(`${borrador.nombre.trim()} was saved.`);
  };

  return (
    <>
      <header className="app__cabeza">
        <div><h1>Customers</h1><p>Who buys from you, from where and how much.</p></div>
        <Boton icono="plus" onClick={() => { setErrores({}); setAbierto(true); }}>New customer</Boton>
      </header>
      <section className="estadisticas" aria-label="Summary">
        <Estadistica etiqueta="Customers" valor={r.total} />
        <Estadistica etiqueta="Active" valor={r.activos} cambio={12.5} />
        <Estadistica etiqueta="Purchases" valor={r.compras} cambio={-3} serie={HISTORIA.nuevos.map((v, i) => v + HISTORIA.recurrentes[i])} />
        <Estadistica etiqueta="Average per customer" valor={r.promedio.toLocaleString("en", { maximumFractionDigits: 1 })} />
      </section>
      <div className="paneles">
        <Panel titulo="Purchases by city">
          <Barras titulo="Purchases by city" datos={porCiudad(lista)} parte />
        </Panel>
        <Panel titulo="Purchases by month">
          <Lineas titulo="Purchases by month" eje="Month" etiquetas={HISTORIA.meses} series={[
            { nombre: "Returning", valores: HISTORIA.recurrentes }, { nombre: "New customers", valores: HISTORIA.nuevos },
          ]} />
        </Panel>
      </div>
      <Pestanas activa={filtro} alCambiar={setFiltro} pestanas={[
        { id: "all", titulo: "All" }, { id: "active", titulo: "Active" }, { id: "pending", titulo: "Pending" }, { id: "inactive", titulo: "Inactive" },
      ]} />
      <Tabla titulo="Customer list" columnas={columnas} filas={filas} buscarEn={["nombre", "correo", "ciudad"]} />
      <Dialogo abierto={abierto} titulo="New customer" alCerrar={() => setAbierto(false)}
        pie={<><Boton variante="secundario" onClick={() => setAbierto(false)}>Cancel</Boton><Boton onClick={guardar}>Save</Boton></>}>
        <Campo etiqueta="Name" autoComplete="name" value={borrador.nombre} error={errores.nombre} onChange={(e) => setBorrador({ ...borrador, nombre: e.target.value })} />
        <Campo etiqueta="Email" type="email" autoComplete="email" value={borrador.correo} error={errores.correo} onChange={(e) => setBorrador({ ...borrador, correo: e.target.value })} />
        <Campo etiqueta="City" value={borrador.ciudad} error={errores.ciudad} onChange={(e) => setBorrador({ ...borrador, ciudad: e.target.value })} />
      </Dialogo>
    </>
  );
}

export default function App() {
  return (
    <Avisos>
      <Aplicacion marca={__MARCA__} actual="clientes" enlaces={[
        { id: "inicio", texto: "Home", icono: "house" },
        { id: "clientes", texto: "Customers", icono: "users" },
        { id: "ventas", texto: "Sales", icono: "chart-column" },
        { id: "ajustes", texto: "Settings", icono: "settings" },
      ]}>
        <Clientes />
      </Aplicacion>
    </Avisos>
  );
}
