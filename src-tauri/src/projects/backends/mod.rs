//! Per-backend knowledge for the agents-v2 spawn path (projects/spawn.rs).
//!
//! `shared` holds the backend-neutral launch helpers that used to live in
//! `team::backends_shared` (board #100): per-backend MCP rendering, the
//! launch-script pattern (the 2KB tty lesson) and startup-prompt confirmation.
//! Moved here so the spawn path owns its dependencies and the desktop Team
//! system can be deleted whole (docs/todo.md §A).

pub(crate) mod shared;
