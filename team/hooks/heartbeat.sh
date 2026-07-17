#!/usr/bin/env bash
#
# heartbeat.sh — liveness pulse + bounded tool lease for team agents.
#
# Modes:
#   pre    pulse now, then pulse every 30s until the matching post hook
#   post   stop the lease and pulse once
#   pulse  pulse once (used by userPromptSubmit)
#
# It deliberately IGNORES the team bus's own coordination tools (wait / post /
# list_agents / read_history). Those are MCP tool calls too, so their postToolUse
# would fire a heartbeat and flip the agent to `working` the instant a `wait`
# returns — overwriting the brief `thinking` state set when a message is
# delivered. Skipping them lets `thinking` show until the agent runs a REAL
# work tool (read/write/shell/...), which is what `working` should mean.
#
# Identity + endpoint are injected by the supervisor at launch (TEAM_HB_URL /
# TEAM_AGENT / TEAM_ROOM). Curl is fire-and-forget with a hard 2s cap. A lost
# post hook cannot keep an agent falsely alive forever: every lease expires
# after 24h.
#
mode="${1:-pulse}"
payload=""
IFS= read -r payload || true        # hook event JSON is one line; never wait for EOF
[ -n "${TEAM_HB_URL:-}" ] || exit 0
command -v curl >/dev/null 2>&1 || exit 0

# Skip the bus's own tools: tool_name like "@team/wait" (kiro) or
# "mcp__team__wait" (claude). A fresh prompt (userPromptSubmit) carries no
# tool_name and falls through — it's about to do real work.
tool="$(printf '%s' "$payload" | grep -oE '"tool_name"[[:space:]]*:[[:space:]]*"[^"]+"' | head -n1)"
case "$tool" in
  *'@team/'*|*'team__'*) exit 0 ;;
esac

heartbeat() {
  curl -s -m 2 -X POST "$TEAM_HB_URL" \
    -H "x-agent: ${TEAM_AGENT:-}" \
    -H "x-room: ${TEAM_ROOM:-}" >/dev/null 2>&1
}

script_dir="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
agent_key="$(printf '%s' "${TEAM_AGENT:-agent}" | tr -c 'A-Za-z0-9_.-' '_')"
lease="$script_dir/.heartbeat-${agent_key}.lease"
pidfile="$script_dir/.heartbeat-${agent_key}.pid"

case "$mode" in
  pre)
    date +%s >"$lease"
    heartbeat &

    if [ -s "$pidfile" ] && kill -0 "$(cat "$pidfile")" 2>/dev/null; then
      exit 0
    fi

    (
      trap 'rm -f "$pidfile"' EXIT
      while [ -f "$lease" ]; do
        sleep 30
        [ -f "$lease" ] || break
        started="$(cat "$lease" 2>/dev/null || echo 0)"
        now="$(date +%s)"
        if [ "$started" -le 0 ] 2>/dev/null || [ $((now - started)) -ge 86400 ]; then
          rm -f "$lease"
          break
        fi
        heartbeat
      done
    ) >/dev/null 2>&1 &
    echo "$!" >"$pidfile"
    ;;
  post)
    rm -f "$lease"
    heartbeat &
    ;;
  pulse)
    heartbeat &
    ;;
esac

exit 0
