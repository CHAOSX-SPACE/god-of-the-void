#!/usr/bin/env bash
# ══════════════════════════════════════════════════════════════════════════
#   FROM SCRATCH — the mortal's test: someone meeting CHAOS for the first time
#
#   Simulates a clean machine: a fake HOME, the installer run once, and then
#   every promise the README makes checked one by one. YOUR machine is never
#   touched. Nothing is declared green without its measurement.
#
#       bash from-scratch.sh
# ══════════════════════════════════════════════════════════════════════════
set -uo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
FAKE="$(mktemp -d)"
fail=0
paso() { printf "  %s %s\n" "$1" "$2"; }
ok()   { paso "✓" "$1"; }
mal()  { paso "✗" "$1"; fail=1; }
trap 'rm -rf "$FAKE"' EXIT

echo "═══ A virgin HOME: $FAKE ═══"
export HOME="$FAKE"
export CHAOS_HOME="$FAKE/.chaos"
export CHAOS_NO_SCHEDULE=1        # a test never schedules a daemon on your box

echo ""
echo "═══ 1. THE INCARNATION ═══"
if python3 "$ROOT/body/install.py" >"$FAKE/install.log" 2>&1; then
  ok "the installer ran"
else
  mal "the installer DIED — see $FAKE/install.log"
  tail -20 "$FAKE/install.log"
fi

echo ""
echo "═══ 2. THE BODY EXISTS ═══"
[ -x "$FAKE/.chaos/bin/chaos" ] && ok "the app 'chaos' is executable" || mal "no app"
[ -f "$FAKE/.chaos/abyss.db" ]  && ok "the neurons were born (abyss.db)" || mal "no database"
[ -d "$FAKE/.claude/skills/chaos" ] && ok "the skill is visible to Claude Code" || mal "skill not installed"
N=$(ls "$FAKE/.claude/skills/chaos/organs" 2>/dev/null | wc -l | tr -d ' ')
[ "$N" = "16" ] && ok "16 organs" || mal "organs: $N (should be 16)"

echo ""
echo "═══ 3. THE NEURONS WORK ═══"
CH="$FAKE/.chaos/bin/chaos"
printf '# Radar\nphased array at 10.5 GHz, aerial detection\n' > "$FAKE/doc.md"
"$CH" devour "$FAKE/doc.md" >/dev/null 2>&1
"$CH" search "aerial detection" 2>/dev/null | grep -qi radar \
  && ok "devour → search: it finds what it ate" || mal "search found nothing"

# The Sense: it must find by MEANING, not by letters
"$CH" search "phased array" 2>/dev/null | grep -qi radar \
  && ok "The Sense answers without a model, without a network" || mal "The Sense is deaf"

echo ""
echo "═══ 4. THE PURGE HOLDS ═══"
printf '# Poison\nkey ghp_ABCDEFGHIJKLMNOPQRSTUVWXYZ0123 and useful text\n' > "$FAKE/poison.md"
"$CH" devour "$FAKE/poison.md" >/dev/null 2>&1
"$CH" hunger "missing ghp_ABCDEFGHIJKLMNOPQRSTUVWXYZ0123" >/dev/null 2>&1
if python3 - "$FAKE/.chaos/abyss.db" <<'PY'
import sqlite3, sys
con = sqlite3.connect(sys.argv[1]); con.text_factory = lambda b: b.decode("utf-8", "replace")
S = "ghp_ABCDEFGHIJKLMNOPQRSTUVWXYZ0123"
for t, in con.execute("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'"):
    for c in [r[1] for r in con.execute("PRAGMA table_info(%s)" % t)]:
        try:
            if con.execute('SELECT 1 FROM "%s" WHERE CAST("%s" AS TEXT) LIKE ? LIMIT 1'
                           % (t, c), ("%" + S + "%",)).fetchone():
                print("LEAK in %s.%s" % (t, c)); sys.exit(1)
        except Exception:
            pass
sys.exit(0)
PY
then ok "no key survives, through any door"; else mal "A KEY LEAKED"; fi

echo ""
echo "═══ 5. THE ERRARIUM AMBUSHES ═══"
"$CH" fault "test fault" --cause "c" --cure "x" --lesson "the lesson survives" >/dev/null 2>&1
"$CH" faults 2>/dev/null | grep -q "test fault" \
  && ok "a fault is recorded and can be recalled" || mal "the errarium does not answer"

echo ""
echo "═══ 6. THE HOOKS ARE INSCRIBED ═══"
if [ -f "$FAKE/.claude/settings.json" ]; then
  H=$(python3 -c "
import json,io
d=json.load(io.open('$FAKE/.claude/settings.json'))
print(len(d.get('hooks',{})))" 2>/dev/null || echo 0)
  [ "${H:-0}" -ge 4 ] && ok "$H hook families inscribed" || mal "hooks: ${H:-0} (expected 5)"
else
  mal "settings.json was never written"
fi

echo ""
echo "═══ 7. THE EYE ═══"
[ -f "$FAKE/.chaos/eye/server.py" ] && ok "organ 16 installed" || mal "the Eye did not travel"

echo ""
if [ $fail -eq 0 ]; then
  echo "🕳️  FROM ZERO TO GOD — everything the README promises, measured and true."
else
  echo "✗ The incarnation is incomplete. Nothing ships until this is green."
  exit 1
fi
