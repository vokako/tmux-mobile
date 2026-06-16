#!/usr/bin/env bash
#
# heartbeat.sh — a liveness ping for team agents. Wired on the per-tool /
# per-prompt hooks (postToolUse, userPromptSubmit), it tells the team daemon
# "I'm alive and working" WHILE an agent is heads-down on a task — i.e. between
# `wait` calls, exactly the window where presence would otherwise go stale and
# the agent be misflagged `unreachable`.
#
# Identity + endpoint are injected by the supervisor at launch (TEAM_HB_URL /
# TEAM_AGENT / TEAM_ROOM). It is deliberately fire-and-forget: the curl runs
# backgrounded with a hard 2s cap and the script exits 0 immediately, so it can
# NEVER block or fail the agent's turn. No-op if the env or curl is missing.
#
cat >/dev/null 2>&1 || true        # drain the hook event JSON on stdin
[ -n "${TEAM_HB_URL:-}" ] || exit 0
command -v curl >/dev/null 2>&1 || exit 0
( curl -s -m 2 -X POST "$TEAM_HB_URL" \
    -H "x-agent: ${TEAM_AGENT:-}" \
    -H "x-room: ${TEAM_ROOM:-}" >/dev/null 2>&1 ) &
exit 0
