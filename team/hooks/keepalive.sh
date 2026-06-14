#!/usr/bin/env bash
#
# keepalive.sh — a Stop hook (Kiro & Claude Code) that nudges a dropped-out agent
# back into its wait loop. It fires only when an agent ENDS its turn without parking
# in `wait` (i.e. it fell out of the loop). Using $TMUX_PANE it re-prompts its own
# pane, so it is fully self-contained. No-op outside tmux.
#
cat >/dev/null 2>&1 || true        # drain the hook event JSON on stdin
[ -n "${TMUX_PANE:-}" ] || exit 0
# Briefly wait for the TUI to return to its prompt, then nudge. Background + exit 0
# immediately so we never block the agent's turn lifecycle.
( sleep 0.6; tmux send-keys -t "$TMUX_PANE" "继续：调用 wait 等待群里的消息，不要主动停止。" Enter ) >/dev/null 2>&1 &
exit 0
