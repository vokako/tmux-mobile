//! The one shell quoter (board #125). A LEAF module — no crate::projects or
//! other gated dependency — so it compiles on every target, Android included
//! (`agent_notifications` is its consumer there).
//!
//! Two forms share ONE escaping core, and the pair is deliberate: they cannot
//! be merged byte-wise. `quote_always` is the form the hook files on disk
//! carry — `patch_hooks`/`refresh_hooks` compare those strings verbatim, so
//! switching an already-spawned agent's config to the minimal form would mark
//! every hook stale once. `quote` is the minimal form launch lines and task
//! commands read best in. The union table below pins both against every
//! vector the three pre-#125 quoters had accumulated.

/// The escaping core: wrap in single quotes, closing around each embedded
/// single quote (`'` → `'\''`). POSIX-shell safe for any byte sequence.
fn escaped(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

/// ALWAYS quotes. The byte-pinned hook-file form (see module doc).
pub fn quote_always(value: &str) -> String {
    escaped(value)
}

/// Quote only when needed. The safe set is the NARROW one the launcher family
/// used (`[A-Za-z0-9] - _ . / = :`): quoting more is always safe, and the
/// wider `,@+` set the old tasks quoter passed bare bought nothing but a
/// second definition (its own test vectors never exercised those bytes).
/// Empty is `''` — a vanished argv element otherwise.
pub fn quote(value: &str) -> String {
    if value.is_empty() {
        return "''".to_string();
    }
    if value
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'/' | b'=' | b':'))
    {
        return value.to_string();
    }
    escaped(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The UNION of every vector the three pre-#125 quoters were tested on,
    /// plus the boundary bytes their safe sets disagreed about. Each row pins
    /// BOTH forms, so a change to either is a deliberate one.
    #[test]
    fn the_union_table_pins_both_forms() {
        // (input, quote(), quote_always())
        let table: &[(&str, &str, &str)] = &[
            ("kiro-cli", "kiro-cli", "'kiro-cli'"),
            ("a/b_c.d", "a/b_c.d", "'a/b_c.d'"),
            ("npm", "npm", "'npm'"),
            ("tauri:dev:release", "tauri:dev:release", "'tauri:dev:release'"),
            ("--release", "--release", "'--release'"),
            ("hello world", "'hello world'", "'hello world'"),
            ("a b", "'a b'", "'a b'"),
            ("你是「经理」", "'你是「经理」'", "'你是「经理」'"),
            (
                "global.anthropic.claude-fable-5-1[1m]",
                "'global.anthropic.claude-fable-5-1[1m]'",
                "'global.anthropic.claude-fable-5-1[1m]'",
            ),
            ("it's", "'it'\\''s'", "'it'\\''s'"),
            ("", "''", "''"),
            ("; rm -rf /", "'; rm -rf /'", "'; rm -rf /'"),
            ("$(whoami)", "'$(whoami)'", "'$(whoami)'"),
            ("50%", "'50%'", "'50%'"),
            // The old tasks safe set passed these bare; quoting them is the
            // equivalent shell word and one definition fewer (board #125).
            ("a,b", "'a,b'", "'a,b'"),
            ("user@host", "'user@host'", "'user@host'"),
            ("c++", "'c++'", "'c++'"),
        ];
        for (input, min, always) in table {
            assert_eq!(&quote(input), min, "quote({input:?})");
            assert_eq!(&quote_always(input), always, "quote_always({input:?})");
        }
    }
}
