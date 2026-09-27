//! What an agent's name is, and how a chat body addresses one (board #248).
//! A LEAF module, no gated dependency, so the phone's server and the `tmm`
//! binary read addresses by the same rule as the desktop.
//!
//! `name_char` is the ONE definition of a name character: `projects::agents::
//! valid_name` builds names from it and `mention_names` reads addresses with
//! it, so a name can never hold a character the parser would cut, and the
//! parser can never read past a name (orchestrator, #248).

/// A character an agent name may contain: a letter or digit of any script
/// (a Chinese agent name is an ordinary name), `-` or `_`. No `.`: a name is
/// also a tmux target and an `@address`, where a dot is a separator/host.
pub fn name_char(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '-' | '_')
}

/// The `@` addresses in a chat body — the ONE server reading of who a message
/// names (board #248). `deliver_mentions`, the room's stored `to`, and the
/// `tmm send` "has a recipient" check all use it; the client's
/// `mentionTokens` (hub.ts) mirrors it case for case.
///
/// Two edges, both from `name_char` — the same set `valid_name` admits:
///
/// * The `@` must START a word: at the start of the body, or after anything
///   that is not an email/host character (ASCII letter or digit, `_`, `.`,
///   `-`). So `me@bob.dev`, `a@bob` and `pkg@2.4.0` name nobody, while
///   `(@bob)`, `"@bob"`, `/@bob` and a CJK character right before it
///   (`请@bob`, which the owner writes) still address bob.
/// * The address is the run of name characters after it, so trailing
///   punctuation of any kind ends it: `(@bob)`, `@bob，`, `@bob。` and
///   `**@bob**` name bob. A run followed by `.` plus a name character, or by
///   another `@`, is a host or an address, not a name (`@bob.dev`, `@a@b`).
///
/// Before #248 every `@` split and the address ran to the next whitespace, so
/// `mail a@bob` typed into bob's pane while `(@bob)` and `@bob，` reached
/// nobody.
pub fn mention_names(body: &str) -> Vec<String> {
    let in_word = |c: char| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-');
    let mut out = Vec::new();
    for (i, _) in body.match_indices('@') {
        if body[..i].chars().next_back().is_some_and(in_word) {
            continue;
        }
        let rest = &body[i + 1..];
        let end = rest.find(|c: char| !name_char(c)).unwrap_or(rest.len());
        let mut after = rest[end..].chars();
        let host = match after.next() {
            Some('@') => true,
            Some('.') => after.next().is_some_and(name_char),
            _ => false,
        };
        if end > 0 && !host {
            out.push(rest[..end].to_string());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Board #248: the ONE case table for both sides — `hub.test.ts` reads
    /// these rows out of this file and runs the client's `mentionTokens` over
    /// them, so keep each row on one line, `("<body>", vec![…]),`.
    #[test]
    fn an_address_must_start_a_word() {
        for (body, tokens) in [
            ("@bob look", vec!["bob"]),
            ("look @bob", vec!["bob"]),
            ("@bob: now, @alice.", vec!["bob", "alice"]),
            ("(@bob) and \"@alice\" and **@carol**", vec!["bob", "alice", "carol"]),
            ("mail me at a@bob.dev", vec![]),
            ("a@bob", vec![]),
            ("x.y@bob and first-last@bob", vec![]),
            ("npm i pkg@2.4.0", vec![]),
            ("see @bob.dev", vec![]),
            ("请@bob 看看，@alice，不急。@builder-2。", vec!["bob", "alice", "builder-2"]),
            ("@kiro/@claude", vec!["kiro", "claude"]),
            ("@a@b", vec![]),
            ("@ alone, @, @!", vec![]),
            ("line one\n@bob line two", vec!["bob"]),
            ("@all standup", vec!["all"]),
            // Unicode parity with the client (validator, #248): a non-BMP
            // letter after `.` is a host; a combining vowel sign is Alphabetic.
            ("@bob.𐐀", vec![]),
            ("see @bob.𐐀 and @राम, ok", vec!["राम"]),
            ("@𐐀𐐁 hi", vec!["𐐀𐐁"]),
            // U+0345 (combining ypogegrammeni) is Alphabetic but neither a
            // Letter nor a Number: a name character here and on the client.
            ("@bob.ͅ x", vec![]),
            ("@bobͅ hi", vec!["bobͅ"]),
        ] {
            assert_eq!(mention_names(body), tokens, "{body:?}");
        }
    }
}
