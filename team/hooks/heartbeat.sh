#!/usr/bin/env bash
#
# heartbeat.sh — a liveness ping for team agents. Wired on the per-tool /
# per-prompt hooks (postToolUse, userPromptSubmit), it tells the team daemon
# "I'm alive and working" WHILE an agent is heads-down on a task — i.e. between
# `wait` calls, exactly the window where presence would otherwise go stale and
# the agent be misflagged.
#
# It deliberately IGNORES the team bus's own coordination tools (wait / post /
# list_agents / history). Those are MCP tool calls too, so their postToolUse
# would fire a heartbeat and flip the agent to `working` the instant a `wait`
# returns — overwriting the brief `thinking` state set when a message is
# delivered. Skipping them lets `thinking` show until the agent runs a REAL
# work tool (read/write/shell/...), which is what `working` should mean.
#
# Identity + endpoint are injected by the supervisor at launch (TEAM_HB_URL /
# TEAM_AGENT / TEAM_ROOM). Fire-and-forget: the curl runs backgrounded with a
# hard 2s cap and the script exits 0 immediately, so it can never block or fail
# the agent's turn. No-op if the env or curl is missing.
#
payload="$(cat 2>/dev/null)"        # the hook event JSON on stdin
[ -n "${TEAM_HB_URL:-}" ] || exit 0
command -v curl >/dev/null 2>&1 || exit 0

# Skip the bus's own tools: tool_name like "@team/wait" (kiro) or
# "mcp__team__wait" (claude). A fresh prompt (userPromptSubmit) carries no
# tool_name and falls through — it's about to do real work.
tool="$(printf '%s' "$payload" | grep -oE '"tool_name"[[:space:]]*:[[:space:]]*"[^"]+"' | head -n1)"
case "$tool" in
  *'@team/'*|*'team__'*) exit 0 ;;
esac

( curl -s -m 2 -X POST "$TEAM_HB_URL" \
    -H "x-agent: ${TEAM_AGENT:-}" \
    -H "x-room: ${TEAM_ROOM:-}" >/dev/null 2>&1 ) &
exit 0
