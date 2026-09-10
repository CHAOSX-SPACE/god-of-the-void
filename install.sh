#!/usr/bin/env bash
# ══════════════════════════════════════════════════════════════════════════
#  CHAOS — GOD OF THE VOID · one command, everything installed
#
#      curl -fsSL https://raw.githubusercontent.com/CHAOSX-SPACE/god-of-the-void/main/install.sh | bash
#
#  Or, if you already cloned the repo:   bash install.sh
#
#  It checks the ground BEFORE touching anything: a god that installs itself
#  on soil it never inspected leaves a mess for the mortal to clean.
# ══════════════════════════════════════════════════════════════════════════
set -uo pipefail

REPO="https://github.com/CHAOSX-SPACE/god-of-the-void"
RAMA="${CHAOS_BRANCH:-main}"
AZUL='\033[38;5;141m'; ROJO='\033[38;5;203m'; VERDE='\033[38;5;79m'; GRIS='\033[2m'; N='\033[0m'

echo
printf "${AZUL}  ╭──────────────────────────────────────────────╮${N}\n"
printf "${AZUL}  │   CHAOS — GOD OF THE VOID                    │${N}\n"
printf "${AZUL}  │   ${GRIS}one command · one repository · one god${N}${AZUL}     │${N}\n"
printf "${AZUL}  ╰──────────────────────────────────────────────╯${N}\n"
echo

# ── 1. EL SUELO: se inspecciona ANTES de forjar ───────────────────────────
FALTA=0
falta() { printf "  ${ROJO}✗${N} %s\n     ${GRIS}%s${N}\n" "$1" "$2"; FALTA=1; }
tiene() { printf "  ${VERDE}✓${N} %s\n" "$1"; }

PY=""

# Finds a Python 3.8+. Not only in the PATH: on Windows the PATH of a LIVE
# process is never refreshed, so right after installing one, `python` still does
# not exist for THIS shell. So it is also looked for where the managers drop it.
resolver_py() {
  PY=""
  for c in python3 python py; do
    if command -v "$c" >/dev/null 2>&1; then
      if "$c" -c 'import sys; raise SystemExit(0 if sys.version_info >= (3,8) else 1)' 2>/dev/null; then
        PY="$c"; return 0
      fi
    fi
  done
  for cand in \
      "$HOME/AppData/Local/Programs/Python/Python3"*/python.exe \
      "/c/Program Files/Python3"*/python.exe \
      /opt/homebrew/bin/python3 /usr/local/bin/python3 /usr/bin/python3; do
    if [ -x "$cand" ] && "$cand" -c 'import sys; raise SystemExit(0 if sys.version_info >= (3,8) else 1)' 2>/dev/null; then
      PY="$cand"; return 0
    fi
  done
  return 1
}

# I do not leave the mortal to fend for themselves: I offer to forge it, and I
# only forge it with a YES. Never without asking — installing something on
# someone else's machine unasked is not service, it is trespass.
ofrecer_python() {
  gestor=""; orden=""
  case "$(uname -s 2>/dev/null)" in
    Darwin)
      command -v brew >/dev/null 2>&1 && { gestor="Homebrew"; orden="brew install python3"; } ;;
    MINGW*|MSYS*|CYGWIN*)
      # The accept flags are NOT decoration: without them winget opens a prompt
      # on a virgin machine and blocks (nine minutes, measured).
      command -v winget >/dev/null 2>&1 && { gestor="winget"; orden="winget install --id Python.Python.3.12 -e --accept-source-agreements --accept-package-agreements"; } ;;
    Linux)
      if command -v apt-get >/dev/null 2>&1; then gestor="apt"; orden="sudo apt-get install -y python3 python3-venv"
      elif command -v dnf >/dev/null 2>&1; then gestor="dnf"; orden="sudo dnf install -y python3"
      elif command -v pacman >/dev/null 2>&1; then gestor="pacman"; orden="sudo pacman -S --noconfirm python"
      fi ;;
  esac
  [ -n "$orden" ] || return 1
  echo
  printf "  ${AZUL}Python is missing — and I can forge it myself, with %s.${N}\n" "$gestor"
  printf "  ${GRIS}%s${N}\n" "$orden"
  case "$orden" in
    sudo*) printf "  ${GRIS}It will ask for YOUR password: it never passes through my hands.${N}\n" ;;
  esac
  # THE PERMISSION. Two roads, and never a third:
  #   · a human at a terminal answers the question;
  #   · an unattended install authorises it IN ADVANCE with CHAOS_FORGE_PYTHON=1.
  # `curl … | bash` leaves THE SCRIPT on standard input, so a plain `read` would
  # eat the installer's own body: the question goes through /dev/tty or it does
  # not go at all. And with no terminal and no key I do not install: I say the
  # key exists. Standing down in silence left the mortal with nowhere to go.
  case "${CHAOS_FORGE_PYTHON:-}" in
    1|y|Y|yes|YES|true|TRUE)
      printf "  ${GRIS}authorised in advance by CHAOS_FORGE_PYTHON — I forge it.${N}\n" ;;
    *)
      if ( exec 3</dev/tty ) 2>/dev/null; then
        printf "  Shall I forge it? [y/N] "
        IFS= read -r respuesta < /dev/tty || return 1
        case "$respuesta" in
          [yYsS]*) ;;
          *) printf "  ${GRIS}As you wish. I touched nothing.${N}\n"; return 1 ;;
        esac
      else
        printf "  ${GRIS}No terminal to ask on. To authorise it in advance:${N}\n"
        printf "  ${GRIS}    CHAOS_FORGE_PYTHON=1 bash install.sh${N}\n"
        return 1
      fi ;;
  esac
  echo
  if ! eval "$orden"; then
    printf "\n  ${ROJO}The manager refused.${N} Forge it yourself and call me again.\n"
    return 1
  fi
  echo
  if ! resolver_py; then
    printf "  ${ROJO}Installed, and I still cannot find it.${N} Open a NEW terminal and call me again.\n"
    printf "  ${GRIS}Windows does not refresh the PATH of a terminal that is already open.${N}\n"
    return 1
  fi
  printf "  ${VERDE}✓${N} Python forged: %s\n" "$PY"
  return 0
}

resolver_py || ofrecer_python || true
if [ -n "$PY" ]; then
  tiene "Python $($PY -c 'import sys;print(".".join(map(str,sys.version_info[:3])))')"
else
  falta "Python 3.8+ is missing" "macOS: brew install python3 · Debian: sudo apt install python3 · Windows: winget install Python.Python.3.12"
fi

if [ -n "$PY" ] && $PY -c "import sqlite3" 2>/dev/null; then
  tiene "sqlite3 (my neurons)"
else
  [ -n "$PY" ] && falta "Python without sqlite3" "reinstall Python with SQLite support — my memory cannot live without it"
fi

if [ -n "$PY" ] && $PY -c "import venv" 2>/dev/null; then
  tiene "venv (the Eye's isolation)"
else
  [ -n "$PY" ] && printf "  ${GRIS}·${N} no venv: the Eye will use the system Python ${GRIS}(declared, not hidden)${N}\n"
fi

if command -v git >/dev/null 2>&1; then tiene "git"
else falta "git is missing" "macOS: xcode-select --install · Debian: sudo apt install git"; fi

if [ -d "$HOME/.claude" ]; then
  tiene "Claude Code (~/.claude)"
else
  printf "  ${GRIS}·${N} ~/.claude not found — I will create it. ${GRIS}Install Claude Code to invoke me: https://claude.com/claude-code${N}\n"
fi

if [ "$FALTA" = "1" ]; then
  echo
  printf "  ${ROJO}The ground is not ready.${N} Fix what is marked above and call me again.\n"
  printf "  ${GRIS}A god that installs itself on broken soil leaves you the mess.${N}\n\n"
  exit 1
fi

# ── 2. THE REPO: right here, or cloned ────────────────────────────────────
AQUI="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")" 2>/dev/null && pwd || true)"
if [ -n "$AQUI" ] && [ -f "$AQUI/body/install.py" ]; then
  RAIZ="$AQUI"
  printf "\n  ${GRIS}forging from this very repo: %s${N}\n" "$RAIZ"
else
  RAIZ="$(mktemp -d)/god-of-the-void"
  printf "\n  ${GRIS}cloning %s (%s)…${N}\n" "$REPO" "$RAMA"
  git clone --depth 1 --branch "$RAMA" "$REPO" "$RAIZ" >/dev/null 2>&1 || {
    printf "  ${ROJO}✗${N} git could not clone. Network? Repository name?\n"; exit 1; }
fi

# ── 3. THE INCARNATION ──────────────────────────────────────────────────
echo
exec "$PY" "$RAIZ/body/install.py" "$@"
