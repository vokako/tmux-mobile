# Chat View Page

> **Status: disabled in UI.** The chat view code (ChatView.svelte, parsers.js,
> i18n keys, markdown/mermaid/katex deps) is all kept, but `chatSupported` in
> `App.svelte` is pinned to `false` so the tab button and tab-swipe target
> never appear. Re-enable by turning `chatSupported` back into `$state(false)`
> and wiring `onChatSupported` back onto the Terminal component.

## Purpose
Renders CLI agent output (Kiro CLI, Claude Code) as a messaging UI. Auto-detected when a supported CLI tool is running in the active pane.

## Components
- User messages → right-aligned bubbles with copy button
- Agent responses → left-aligned bubbles with bot avatar, markdown rendered
- Code blocks → syntax-highlighted cards
- Tool calls → compact collapsible cards
- Diffs → red/green line-by-line rendering
- `/compact` → styled summary card with markdown rendering
- `/model` → interactive model selector (tap to switch)
- Thinking state → debounced spinner animation (CSS, not braille characters)

## Interactions
- Tap code block → copy to clipboard
- Tap tool call card → expand/collapse details
- Tap model option in `/model` selector → sends model switch command
- Scroll → standard scroll through conversation history

## API Calls
- Uses same `subscribe`/`send_command` as Terminal page (shares pane connection)
- No additional API calls — all data comes from parsing terminal output

## State Management
- Parsed messages array (role: user/agent/system, blocks: text/code/tool/diff)
- Parser instance (auto-detected via `detectParser()` from `parsers.js`)
- Thinking spinner visibility (debounced)
- Scroll position

## Edge Cases
- Two parsers exist: Kiro CLI (active, detects by command name) and Claude Code (disabled, `detect()` returns false)
- Parser uses ANSI color codes as semantic markers BEFORE stripping — color 93 = user prompt, color 141 = agent response
- Braille spinner lines filtered and replaced with CSS animation
- System hints (colored text at prompt) distinguished from real user input (no ANSI codes)
- ANSI colors preserved in rendered output with 256-color palette
- Dark color readability adjustment (`ensureReadable`)
- Chat detection runs immediately on session open
