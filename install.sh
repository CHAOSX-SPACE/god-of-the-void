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
for c in python3 python; do
  if command -v "$c" >/dev/null 2>&1; then
    if "$c" -c 'import sys; raise SystemExit(0 if sys.version_info >= (3,8) else 1)' 2>/dev/null; then
      PY="$c"; break
    fi
  fi
done
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
