#!/bin/sh
# ══════════════════════════════════════════════════════════════════════════
#  THE INTERPRETER IS RESOLVED HERE — NEVER IN A JSON.
#
#  A static hooks.json must name one interpreter for three operating systems,
#  and no name serves all three:
#     macOS         has `python3`, and no `python` at all since 12.3
#     Debian/Ubuntu has `python3`, and `python` only with an extra package
#     Windows 11    has the REAL one as `python`, while `python3` is a 0-byte
#                   Microsoft Store ALIAS that is not a Python at all.
#
#  Measured on Windows 11 Pro (build 26100), each candidate timed alone: the
#  alias exits non-zero in about a second, printing an advert for the Store —
#  and in a real console it OPENS the Microsoft Store. So it is stepped over by
#  its ADDRESS, never probed by running it: probing costs a process and can pop
#  a shop window in the Bearer's face. `py` goes first on Windows for the same
#  reason it is trustworthy: the launcher is never a Store alias.
#
#  (An earlier version of this comment claimed the alias HANGS. It does not.
#  That was a timeout of the measuring channel, blamed on the thing measured.)
#
#  Usage:  sh run.sh <hook-file.py>
# ══════════════════════════════════════════════════════════════════════════
AQUI="$(cd "$(dirname "$0")" && pwd)"
GUION="$AQUI/../body/$1"
[ -f "$GUION" ] || exit 0        # a hook that is not there breaks nothing

case "$(uname -s 2>/dev/null)" in
  MINGW*|MSYS*|CYGWIN*) ORDEN="py python python3" ;;
  *)                    ORDEN="python3 python" ;;
esac

PY=""
RESERVA=""
for c in $ORDEN; do
  w="$(command -v "$c" 2>/dev/null)" || continue
  [ -n "$w" ] || continue
  case "$w" in
    *WindowsApps*) [ -n "$RESERVA" ] || RESERVA="$c"; continue ;;   # Store alias: last resort
  esac
  PY="$c"; break
done
[ -n "$PY" ] || PY="$RESERVA"
[ -n "$PY" ] || exit 0           # no Python: I fall silent, I do not break the session

shift
exec "$PY" "$GUION" "$@"
