#!/usr/bin/env bash
# Board #323 smoke: tmm gateway install/status/restart/uninstall under the
# TEST service identity, a scratch config root and port 19899 — never the
# production unit or ~/.config (every call goes through T(), which prefixes
# the environment; no exported state to lose). Linux (systemd --user) only.
# Usage: scripts/gateway-smoke.sh <path to a built tmm>
set -eu
S=$(mktemp -d /tmp/gw323.XXXX)
mkdir -p "$S/tmux-mobile"
printf 'port = 19899\nhost = "127.0.0.1"\n' > "$S/tmux-mobile/config.toml"
cp "$1" "$S/tmm"
T() { env -u PORT -u HOST XDG_CONFIG_HOME="$S" TMM_GATEWAY_SERVICE=tmux-mobile-gateway-test.service "$S/tmm" "$@"; }
[ "$(env XDG_CONFIG_HOME="$S" sh -c 'echo $XDG_CONFIG_HOME')" = "$S" ] || { echo "env not applied"; exit 9; }
echo "--- install"; T gateway install
U=~/.config/systemd/user/tmux-mobile-gateway-test.service
grep -q "XDG_CONFIG_HOME=$S\"" "$U" && echo "unit carries the scratch root"
sleep 2
echo "--- the service reads the scratch config"; journalctl --user -u tmux-mobile-gateway-test.service -n 5 --no-pager | grep -o "listening on .*"
echo "--- status"; T gateway status; echo "status rc=$?"
echo "--- idempotent: same bytes and running, the pid is unchanged"
P1=$(systemctl --user show tmux-mobile-gateway-test.service -p MainPID --value); T gateway install >/dev/null 2>&1; P2=$(systemctl --user show tmux-mobile-gateway-test.service -p MainPID --value); [ "$P1" = "$P2" ] && echo "pid $P1 kept"
echo "--- restart is the forced one"; T gateway restart; sleep 2; P3=$(systemctl --user show tmux-mobile-gateway-test.service -p MainPID --value); [ "$P3" != "$P1" ] && echo "pid $P1 -> $P3"
echo "--- a second foreground start says who holds the port"; (T gateway start 2>&1 || true) | tail -1
echo "--- another tmm (different exe) is refused"; cp "$S/tmm" "$S/tmm2"
env -u PORT -u HOST XDG_CONFIG_HOME="$S" TMM_GATEWAY_SERVICE=tmux-mobile-gateway-test.service "$S/tmm2" gateway uninstall 2>&1 | tail -1 || true
echo "--- uninstall ours"; T gateway uninstall
systemctl --user is-active tmux-mobile-gateway-test.service || true
[ -e "$U" ] && echo "unit still there!" || echo "unit removed"
rm -rf "$S"
