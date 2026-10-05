//! Sessions header control chrome and coarse-pointer hit targets.

use std::path::Path;

fn read(rel: &str) -> String {
    let path = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../..")).join(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

/// Strip `/* … */` comments so property assertions never match commentary.
fn strip_css_comments(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(open) = rest.find("/*") {
        out.push_str(&rest[..open]);
        if let Some(close) = rest[open + 2..].find("*/") {
            rest = &rest[open + 2 + close + 2..];
        } else {
            break;
        }
    }
    out.push_str(rest);
    out
}

/// Inner text of the brace-matched block starting at `open` (index of `{`).
fn brace_body(bytes: &[u8], open: usize) -> String {
    let mut depth = 0usize;
    let mut i = open;
    while i < bytes.len() {
        match bytes[i] {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return String::from_utf8_lossy(&bytes[open + 1..i]).into_owned();
                }
            }
            _ => {}
        }
        i += 1;
    }
    panic!("unbalanced braces from offset {open}");
}

/// The declaration block for the rule whose selector is EXACTLY `selector`
/// (next non-whitespace char after the selector is `{`).
fn rule_body_exact(css: &str, selector: &str) -> String {
    let css = strip_css_comments(css);
    let bytes = css.as_bytes();
    let mut from = 0usize;
    while let Some(rel) = css[from..].find(selector) {
        let at = from + rel;
        let after = &css[at + selector.len()..];
        let trimmed = after.trim_start();
        if trimmed.starts_with('{') {
            let open = at + selector.len() + (after.len() - trimmed.len());
            return brace_body(bytes, open);
        }
        from = at + selector.len();
    }
    panic!("no rule with exact selector `{selector}` in chrome.css");
}

// ---- 2. Sessions button carries the app-chrome idiom ------------------------

fn chrome_css() -> String {
    read("styles/chrome.css")
}

#[test]
fn sessions_trigger_has_hairline_border_and_centered_label() {
    let css = chrome_css();
    let body = rule_body_exact(&css, ".ocean-sessions-trigger");
    // Centered label — the operator-reported alignment issue.
    assert!(
        body.contains("justify-content: center"),
        "the Sessions button must center its label (justify-content:center) — \
         the reported alignment defect (TASK-14); rule body:\n{body}",
    );
    // Hairline edge from the shared border token — the desktop-app cue.
    assert!(
        body.contains("border: 1px solid var(--border)"),
        "the Sessions button must carry a hairline border (1px solid \
         var(--border)) like its neighbouring header controls, not a borderless \
         fill (TASK-14); rule body:\n{body}",
    );
    assert!(
        !body.contains("border: none"),
        "the flat borderless treatment must be gone — it read as a cheap web \
         button (TASK-14)",
    );
    // Elevation + radius stay on the repo tokens (no invented values).
    assert!(
        body.contains("box-shadow: var(--elev-1)") && body.contains("border-radius: var(--radius)"),
        "the Sessions button must keep the repo elevation/radius tokens \
         (--elev-1 / --radius) — TASK-14",
    );
}

#[test]
fn sessions_trigger_hover_strengthens_edge() {
    let css = chrome_css();
    let hover = rule_body_exact(&css, ".ocean-sessions-trigger:hover");
    assert!(
        hover.contains("background: var(--bg-hover)")
            && hover.contains("border-color: var(--border-strong)"),
        "Sessions hover must lift the fill AND strengthen the edge \
         (--bg-hover + --border-strong) — matches the --thinking/--model-override \
         hover idiom (TASK-14); hover body:\n{hover}",
    );
}

#[test]
fn sessions_trigger_has_coarse_pointer_hit_floor() {
    let css = strip_css_comments(&chrome_css());
    // A coarse-pointer rule grows the tap target via an invisible ::after,
    // reaching the 44px TASK-42 floor without disturbing desktop density.
    assert!(
        css.contains("(pointer: coarse)") && css.contains(".ocean-sessions-trigger::after"),
        "the Sessions button must gain a coarse-pointer hit floor via \
         .ocean-sessions-trigger::after inside a (pointer: coarse) query \
         (TASK-42 idiom) — TASK-14",
    );
}
