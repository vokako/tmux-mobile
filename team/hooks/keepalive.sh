#!/usr/bin/env bash
#
# keepalive.sh — a Stop hook (Kiro, Claude Code, and Codex) that nudges a dropped-out agent
# back into its wait loop. It fires only when an agent ENDS its turn without parking
# in `wait` (i.e. it fell out of the loop). Using $TMUX_PANE it re-prompts its own
# pane, so it is fully self-contained. No-op outside tmux.
#
IFS= read -r _hook_payload || true  # consume one JSON line; never wait for EOF
[ -n "${TMUX_PANE:-}" ] || exit 0
# Briefly wait for the TUI to return to its prompt, then type and submit in
# separate steps. Codex can drop Enter when text + Enter arrive in one tmux
# command during its turn-complete redraw. Background + exit 0 immediately so
# we never block the agent's turn lifecycle.
(
  sleep 0.6
  tmux send-keys -t "$TMUX_PANE" -l "Continue: call \`wait\`."
  sleep 0.3
  tmux send-keys -t "$TMUX_PANE" Enter
) >/dev/null 2>&1 &
exit 0
