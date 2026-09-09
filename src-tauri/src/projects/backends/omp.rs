//! The omp backend's own knowledge (board #101/#127): every fact about how
//! this CLI is driven lives here — one file to touch when it changes.

/// omp 18.0.6 `--thinking` (its own --help): "off, minimal, low, medium,
/// high, xhigh, max, auto".
pub(crate) fn effort_values() -> &'static [&'static str] {
    &["off", "minimal", "low", "medium", "high", "xhigh", "max", "auto"]
}

/// omp models ride its own catalog files; no authoritative list to ask.
pub(crate) fn models_fetch() -> Option<Vec<String>> {
    None
}

use crate::projects::vitals::Vitals;

/// omp's persistent footer is the TOP border of its input box — one line
/// carrying the π mark and, width permitting, the model, thinking level,
/// cwd, session cost and a context gauge (measured, omp 18.0.6):
///
/// `╭── π  > ⬢ Fable 5.1 (Bedrock, 1M) · ◒ high > 📁 /path > $0.45 ▶─3%─┃1M───╮`
///
/// The line is RESPONSIVE: a fresh session has no gauge yet
/// (`… > 📁 /path ▶────────╮`), and a narrow pane drops the model/effort
/// segments entirely (`╭── π  > 📁 …work ▶────13%───┃────1M───╮` was
/// measured live) — so every field is independently optional and `backfill`
/// carries an older wide reading across a narrow capture. The `>`-separated
/// segments are read by MARK, not position: `⬢` heads the model (with the
/// thinking level as its `·` sub-segment), `▶ … ┃` frames the used-context
/// percentage. Cost and the window size have no Vitals field and are not
/// read.
pub fn sniff_omp(pane: &str) -> Vitals {
    let mut v = Vitals::default();
    for line in pane.lines().rev() {
        let s = line.trim();
        // The signature: an input-box top border that carries omp's π mark.
        // Tool cards and plain output draw boxes too, but never with π.
        if !(s.starts_with('╭') && s.ends_with('╮') && s.contains(" π ")) {
            continue;
        }
        for segment in s.split(" > ") {
            let segment = segment.trim();
            if let Some(model_part) = segment.strip_prefix('⬢') {
                // `⬢ <model> [· <glyph> <effort>]` — the glyph varies with
                // the level, so the effort is read as the sub-segment's last
                // word, gated on omp's own enum.
                let mut parts = model_part.split(" · ");
                let model = parts.next().unwrap_or("").trim();
                if !model.is_empty() {
                    v.model = Some(model.to_string());
                    // The model segment is the one that carries the level:
                    // seeing it without one is a verdict, not a truncation.
                    v.effort_definitive = true;
                }
                for extra in parts {
                    if let Some(word) = extra.split_whitespace().last() {
                        if matches!(word, "off" | "minimal" | "low" | "medium" | "high" | "xhigh" | "max" | "auto") {
                            v.effort = Some(word.to_string());
                        }
                    }
                }
            }
        }
        // `▶ … NN% … ┃` — the gauge frames the used share. The ┃ (or the
        // closing ╮ on a gauge that has no limit mark yet) bounds the scan so
        // a percentage in the cwd segment can never be read as context.
        if let Some(bar) = s.find('▶').map(|i| &s[i..]) {
            let bar = bar.split('┃').next().unwrap_or(bar);
            if let Some(end) = bar.find('%') {
                let digits: String = bar[..end]
                    .chars()
                    .rev()
                    .take_while(char::is_ascii_digit)
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .collect();
                if let Ok(pct) = digits.parse::<u8>() {
                    if pct <= 100 {
                        v.context_pct = Some(pct);
                    }
                }
            }
        }
        break; // the last π border is the live footer; older ones scrolled by
    }
    v
}

