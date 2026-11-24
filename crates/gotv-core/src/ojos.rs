//! K1 · Los OJOS del sistema: órdenes de INSPECCIÓN de la Mac que sólo leen y que la caja de
//! arena bloquea sin necesidad (medido 2026-11-11 en la charla de voz del Portador: «¿qué
//! procesador tiene?» → `sysctl` bloqueado por la caja, el aviso decía «escritura» y el modelo
//! terminó pidiendo `sudo`). Éstas corren FUERA de la caja y cuentan como LECTURA (no preguntan).
//!
//! La lista es CERRADA y por argumento: un programa entra sólo con sus formas de mirar
//! (`sysctl` sin `-w` ni `nombre=valor`, `top` sólo con `-l`, `diskutil list|info`…). Las
//! tuberías hacia filtros de texto se aceptan; redirecciones, `$(…)`, comillas invertidas,
//! `&` sueltos y `osascript` (puede hacer cualquier cosa) no.

/// Programas que sólo muestran y cuyos argumentos no cambian nada.
const SIEMPRE: &[&str] = &[
    "ps", "pgrep", "lsof", "uptime", "who", "w", "whoami", "id", "hostname", "uname", "sw_vers", "system_profiler", "ioreg", "vm_stat",
    "iostat", "df", "du", "date", "netstat", "ifconfig", "arp", "mdfind", "mdls", "kextstat", "nproc", "groups", "last", "sysctl", "top",
    "pmset", "diskutil", "networksetup", "scutil", "launchctl", "log", "defaults", "csrutil", "spctl", "fdesetup", "tmutil", "nvram",
    "system_profiler", "softwareupdate", "dscl", "pkgutil", "mount", "sharing", "systemsetup", "ipconfig", "route", "airport", "nettop",
    "powermetrics",
];

/// Filtros de texto que pueden ir DESPUÉS de una tubería.
const FILTROS: &[&str] = &["grep", "egrep", "head", "tail", "sort", "uniq", "wc", "cut", "awk", "sed", "tr", "column", "less", "cat", "xargs"];

fn mira(w: &[&str]) -> bool {
    let Some(p0) = w.first().map(|p| p.rsplit('/').next().unwrap_or(p)) else { return false };
    if !SIEMPRE.contains(&p0) {
        return false;
    }
    let args = &w[1..];
    let hay = |x: &str| args.contains(&x);
    let empieza = |pre: &str| args.iter().any(|a| a.starts_with(pre));
    match p0 {
        // escribir un valor: `-w` o `nombre=valor`
        "sysctl" => !hay("-w") && !args.iter().any(|a| a.contains('=')),
        // `top` sin `-l` es interactivo (se queda esperando)
        "top" => empieza("-l"),
        "pmset" => args.first() == Some(&"-g"),
        "diskutil" => matches!(args.first(), Some(&"list" | &"info" | &"activity")) || (args.first() == Some(&"apfs") && matches!(args.get(1), Some(&"list" | &"listSnapshots"))),
        "networksetup" => args.first().is_some_and(|a| a.starts_with("-get") || a.starts_with("-list") || *a == "-printcommands"),
        "scutil" => args.first().is_some_and(|a| matches!(*a, "--get" | "--dns" | "--proxy" | "--nwi")),
        "launchctl" => matches!(args.first(), Some(&"list" | &"print" | &"print-disabled" | &"blame" | &"dumpstate")),
        "log" => matches!(args.first(), Some(&"show" | &"stats")),
        "defaults" => matches!(args.first(), Some(&"read" | &"read-type" | &"domains" | &"find")),
        "csrutil" | "spctl" | "fdesetup" => matches!(args.first(), Some(&"status" | &"--status" | &"list")),
        "tmutil" => matches!(args.first(), Some(&"status" | &"listbackups" | &"latestbackup" | &"destinationinfo" | &"machinedirectory")),
        "nvram" => args.is_empty() || hay("-p") || (args.len() == 1 && !args[0].contains('=') && !args[0].starts_with('-')),
        "softwareupdate" => hay("--list") || hay("-l") || hay("--history"),
        "dscl" => args.iter().any(|a| matches!(*a, "-read" | "-list" | "-search")),
        "pkgutil" => args.first().is_some_and(|a| matches!(*a, "--pkgs" | "--pkg-info" | "--files" | "--file-info")),
        "systemsetup" => args.first().is_some_and(|a| a.starts_with("-get")),
        "sharing" => hay("-l"),
        "ipconfig" => matches!(args.first(), Some(&"getifaddr" | &"getpacket" | &"getoption" | &"getsummary")),
        "route" => matches!(args.first(), Some(&"get" | &"-n")) && !hay("add") && !hay("delete") && !hay("change"),
        "airport" => hay("-I") || hay("-s"),
        "nettop" | "powermetrics" => empieza("-l") || hay("-n") && empieza("-l"),
        "mount" => args.is_empty(),
        "date" => args.first().is_none_or(|a| a.starts_with('+') || a.starts_with('-')),
        "ifconfig" => args.len() <= 1 || hay("-a") || hay("-l"),
        "arp" => hay("-a") || hay("-n"),
        _ => true,
    }
}

fn filtra(w: &[&str]) -> bool {
    let Some(p0) = w.first().map(|p| p.rsplit('/').next().unwrap_or(p)) else { return false };
    if !FILTROS.contains(&p0) {
        return false;
    }
    let parte = w.join(" ");
    match p0 {
        "sed" => !w.iter().any(|t| t.starts_with("-i")) && !parte.contains("w ") && !parte.contains("/w"),
        "awk" => !parte.contains("system(") && !parte.contains('>') && !parte.contains("getline"),
        // xargs sólo hacia otra orden de mirar o filtro
        "xargs" => {
            let resto: Vec<&str> = w[1..].iter().skip_while(|t| t.starts_with('-')).copied().collect();
            !resto.is_empty() && (mira(&resto) || filtra(&resto))
        }
        _ => true,
    }
}

/// ¿La orden SÓLO inspecciona el sistema? (puede correr fuera de la caja, sin preguntar)
pub fn solo_mira(cmd: &str) -> bool {
    let c = cmd.trim();
    if c.is_empty() || c.contains('>') || c.contains('<') || c.contains('`') || c.contains("$(") || c.contains("${") || c.contains("osascript") {
        return false;
    }
    // los separadores DENTRO de comillas son texto (`grep -E 'a|b'`): se tapan para partir
    // (las comprobaciones de arriba miran la orden ORIGINAL, así que un `>` entre comillas
    // sigue rechazándose: conservador)
    let mut tapada = String::with_capacity(c.len());
    let mut comilla: Option<char> = None;
    for ch in c.chars() {
        match (comilla, ch) {
            (None, '\'' | '"') => {
                comilla = Some(ch);
                tapada.push(ch);
            }
            (Some(q), _) if ch == q => {
                comilla = None;
                tapada.push(ch);
            }
            (Some(_), '|' | ';' | '&' | '\n') => tapada.push(' '),
            _ => tapada.push(ch),
        }
    }
    if comilla.is_some() {
        return false; // comillas sin cerrar: no se adivina
    }
    let c = tapada.as_str();
    // un `&` suelto manda a segundo plano; `&&` encadena
    if c.replace("&&", "").contains('&') {
        return false;
    }
    let mut alguno = false;
    for orden in c.split("&&").flat_map(|x| x.split([';', '\n'])) {
        let tramos: Vec<&str> = orden.split('|').map(str::trim).collect();
        if tramos.iter().any(|t| t.is_empty()) {
            if orden.trim().is_empty() {
                continue;
            }
            return false;
        }
        let palabras: Vec<Vec<&str>> = tramos.iter().map(|t| t.split_whitespace().collect()).collect();
        // la primera mira; las siguientes (tras `|`) sólo filtran o también miran
        if !mira(&palabras[0]) || !palabras[1..].iter().all(|w| filtra(w) || mira(w)) {
            return false;
        }
        alguno = true;
    }
    alguno
}

#[cfg(test)]
mod pruebas {
    use super::*;

    #[test]
    fn mirar_el_sistema_si_cambiarlo_no() {
        for c in [
            "sysctl -n machdep.cpu.brand_string",
            "sysctl hw.memsize hw.ncpu",
            "system_profiler SPHardwareDataType | grep -i chip",
            "ps aux | sort -nrk 3 | head -5",
            "top -l 1 -n 10",
            "diskutil list",
            "diskutil info disk0",
            "pmset -g batt",
            "networksetup -getairportnetwork en0",
            "scutil --get ComputerName",
            "launchctl list | grep gotv",
            "log show --last 5m --predicate 'process == \"kernel\"' | tail -20",
            "defaults read com.apple.dock autohide",
            "csrutil status",
            "df -h / && vm_stat",
            "lsof -i :8080",
            "softwareupdate --list",
            "ifconfig en0",
            "date +%H:%M",
            "mdfind -name informe.pdf | head",
            "tmutil latestbackup",
            // medido (banco K4, 9B): el `|` DENTRO de las comillas del grep la partía mal
            "top -l 1 -n 0 | grep -E 'PhysMem|CPU usage'",
            "ps aux | grep -E \"Chrome|Safari\" | head",
        ] {
            assert!(solo_mira(c), "debería mirar: {c}");
        }
        for c in [
            "sysctl -w kern.maxfiles=1000",
            "sysctl kern.maxfiles=1000",
            "top",
            "pmset -a lowpowermode 1",
            "diskutil eraseDisk APFS X disk2",
            "diskutil unmount disk2",
            "networksetup -setairportpower en0 off",
            "scutil --set ComputerName X",
            "launchctl unload x.plist",
            "launchctl kickstart -k system/x",
            "defaults write com.apple.dock autohide -bool true",
            "log erase --all",
            "ps aux > procesos.txt",
            "ps aux | xargs kill",
            "ps aux | sed -i s/a/b/ x",
            "ps aux | awk '{system(\"rm \" $2)}'",
            "sysctl -n hw.ncpu & rm -rf ~",
            "uptime; rm -rf build",
            "echo $(rm x)",
            "osascript -e 'tell app \"Finder\" to get name of startup disk'",
            "cat /etc/hosts",
            "date 1111111111",
            "nvram boot-args=-v",
            "route add default 1.2.3.4",
            // las comillas no esconden una orden: fuera de ellas sigue el `;`
            "ps aux | grep 'x'; rm -rf build",
            "ps aux | awk '{print $1 > \"out.txt\"}'",
            "",
        ] {
            assert!(!solo_mira(c), "NO debería pasar como mirar: {c}");
        }
    }
}
