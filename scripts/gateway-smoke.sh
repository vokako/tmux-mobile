#!/usr/bin/env bash
# Board #323 smoke: tmm gateway install/status/restart/uninstall on a REAL
# systemd user manager, under a TEST service identity, a scratch config root
# and a free port — never the production unit, never ~/.config.
#
# Safety:
# - the TEST name is unique per run (tmux-mobile-gateway-test-<pid>.service),
#   so two runs never address each other's unit; preflight still refuses
#   when it exists or the port is taken;
# - every tmm call goes through T(): `env -i` with only HOME, PATH, the
#   user-manager bus (XDG_RUNTIME_DIR, DBUS_SESSION_BUS_ADDRESS),
#   XDG_CONFIG_HOME and TMM_GATEWAY_SERVICE, so no override (HOST, PORT,
#   TOKEN, TLS_*, TMUX_SOCKET, …) reaches it;
# - the EXIT trap stops the foreground gateway this run started, then
#   removes the unit ONLY through `tmm gateway uninstall` — the same
#   identity check (this exe, this scratch root) as every verb, so a unit
#   that is not this run's is refused and never stopped. A cleanup that
#   fails makes the run fail, and the scratch root (the exe and config a
#   still-running instance uses) is kept with the evidence;
# - on failure the journal tail and the unit are copied to an evidence dir;
# - every check is an assertion: a failed one fails the run.
#
# Usage: scripts/gateway-smoke.sh <path to a built tmm> [port]
set -euo pipefail
TMM_SRC=${1:?usage: gateway-smoke.sh <tmm> [port]}
PORT=${2:-19899}
NAME=tmux-mobile-gateway-test-$$.service
UNIT=~/.config/systemd/user/$NAME
fail() { echo "SMOKE FAIL: $*" >&2; exit 1; }
ok() { echo "  ok: $*"; }

# ── preflight ────────────────────────────────────────────────────────────────
[ -e "$UNIT" ] && fail "$UNIT already exists — another run, or someone else's; not touching it"
systemctl --user cat "$NAME" >/dev/null 2>&1 && fail "$NAME is known to the user manager already"
ss -ltn "sport = :$PORT" | grep -q LISTEN && fail "port $PORT is in use"

S=$(mktemp -d /tmp/gw323-smoke.XXXX)
INSTALLING=0
FG=
cleanup() {
  rc=$?
  trap - EXIT
  if [ -n "$FG" ]; then kill "$FG" 2>/dev/null || true; wait "$FG" 2>/dev/null || true; fi
  if [ "$INSTALLING" = 1 ]; then
    if [ $rc -ne 0 ]; then
      mkdir -p "$S/evidence"
      journalctl --user -u "$NAME" -n 50 --no-pager > "$S/evidence/journal.txt" 2>&1 || true
      cp "$UNIT" "$S/evidence/" 2>/dev/null || true
    fi
    # Only through tmm's identity check: a unit that is not this run's is
    # refused there and left running.
    if [ -e "$UNIT" ]; then
      if T gateway uninstall > "$S/uninstall.log" 2>&1; then
        # Only after OUR uninstall succeeded: the failed state is this run's.
        systemctl --user reset-failed "$NAME" >/dev/null 2>&1 || true
      else
        echo "SMOKE CLEANUP FAILED: $(tail -1 "$S/uninstall.log")" >&2
        [ $rc -eq 0 ] && rc=1
      fi
    fi
  fi
  if [ $rc -ne 0 ] && [ -d "$S/evidence" ]; then
    cp -r "$S/evidence" "/tmp/gw323-smoke-evidence-$$" && echo "evidence kept in /tmp/gw323-smoke-evidence-$$" >&2
  fi
  if [ -e "$UNIT" ]; then
    echo "$UNIT is still installed — keeping $S (its exe and config)" >&2
    [ $rc -eq 0 ] && rc=1
  else
    rm -rf "$S"
  fi
  if [ $rc -eq 0 ]; then echo "SMOKE OK"; else echo "SMOKE FAILED (rc=$rc)" >&2; fi
  exit $rc
}
trap cleanup EXIT

mkdir -p "$S/tmux-mobile"
printf 'port = %s\nhost = "127.0.0.1"\n' "$PORT" > "$S/tmux-mobile/config.toml"
cp "$TMM_SRC" "$S/tmm"
BASE=(HOME="$HOME" PATH="$PATH" XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR:-/run/user/$(id -u)}" DBUS_SESSION_BUS_ADDRESS="${DBUS_SESSION_BUS_ADDRESS:-unix:path=/run/user/$(id -u)/bus}")
T() { env -i "${BASE[@]}" XDG_CONFIG_HOME="$S" TMM_GATEWAY_SERVICE="$NAME" "$S/tmm" "$@"; }
pid() { systemctl --user show "$NAME" -p MainPID --value; }

echo "--- a managed start ignores the environment (--service reads config.toml alone)"
env -i "${BASE[@]}" XDG_CONFIG_HOME="$S" PORT=1 HOST=9.9.9.9 "$S/tmm" gateway start --service > "$S/fg.log" 2>&1 &
FG=$!
for _ in $(seq 40); do grep -q "listening on" "$S/fg.log" && break; sleep 0.25; done
grep -q "listening on ws://127.0.0.1:$PORT" "$S/fg.log" || { cat "$S/fg.log" >&2; fail "--service did not use config.toml's port"; }
kill "$FG"; wait "$FG" 2>/dev/null || true; FG=
ok "PORT=1 HOST=9.9.9.9 ignored; listened on 127.0.0.1:$PORT"

echo "--- install"
INSTALLING=1
T gateway install
grep -q "XDG_CONFIG_HOME=$S\"" "$UNIT" || fail "the unit does not carry the scratch root"
grep -q " gateway start --service$" "$UNIT" || fail "the unit does not run gateway start --service"
grep -qi token "$UNIT" && fail "a token-ish string is in the unit"
ok "unit carries the scratch root, runs --service, holds no token"
journalctl --user -u "$NAME" -n 20 --no-pager | grep -q "listening on ws://127.0.0.1:$PORT" || fail "the service is not on the scratch config's port"
ok "the service listens on 127.0.0.1:$PORT"

echo "--- status (read-only, exit 0 when ours answers)"
T gateway status
T gateway status | grep -q "answering at ws://127.0.0.1:$PORT (this machine)" || fail "status does not see our gateway"

echo "--- idempotent install keeps the pid"
P1=$(pid); T gateway install >/dev/null; P2=$(pid)
[ "$P1" = "$P2" ] || fail "install restarted a running, unchanged service ($P1 -> $P2)"
ok "pid $P1 kept"

echo "--- restart is the forced one"
T gateway restart; P3=$(pid)
[ "$P3" != "$P1" ] || fail "restart did not restart"
ok "pid $P1 -> $P3"

echo "--- a second foreground start names the holder"
OUT=$(T gateway start 2>&1 || true)
echo "$OUT" | tail -1
echo "$OUT" | grep -q "already answers at ws://127.0.0.1:$PORT" || fail "a second start did not name the holder"

echo "--- another tmm (different executable) is refused, and changes nothing"
cp "$S/tmm" "$S/tmm2"
OUT=$(env -i "${BASE[@]}" XDG_CONFIG_HOME="$S" TMM_GATEWAY_SERVICE="$NAME" "$S/tmm2" gateway uninstall 2>&1 || true)
echo "$OUT" | tail -1
echo "$OUT" | grep -q "not this tmm" || fail "another executable's uninstall was not refused"
[ -e "$UNIT" ] && [ "$(pid)" = "$P3" ] || fail "the refused uninstall changed something"
ok "refused; unit and pid unchanged"

echo "--- an invalid service name is refused"
OUT=$(env -i "${BASE[@]}" XDG_CONFIG_HOME="$S" TMM_GATEWAY_SERVICE="../escape.service" "$S/tmm" gateway status 2>&1 || true)
echo "$OUT" | grep -q "not a valid service name" || fail "a path-escaping name was accepted"
ok "../escape.service refused"

echo "--- uninstall ours"
T gateway uninstall
[ -e "$UNIT" ] && fail "the unit is still there"
systemctl --user is-active --quiet "$NAME" && fail "the service still runs"
ok "removed, stopped"
