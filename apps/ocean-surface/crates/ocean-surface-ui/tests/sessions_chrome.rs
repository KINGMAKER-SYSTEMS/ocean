//! One Sessions entry on every host (team-platform amendment P1).
//!
//! Source-assertion tests (same style as open_transcript_layout.rs /
//! dead_selector_removal.rs). The web-only `.ocean-sessions-trigger` text
//! button is retired: the header Dynamic Island is the Sessions entry on web,
//! extension, and Tauri alike, and host detection may gate only native
//! titlebar chrome — never layout, panels, or navigation.

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

/// Strip `//`-line and `/* */`-block comments from Rust so assertions match
/// real code, never the doc/comment prose that explains the rule.
fn strip_rust_comments(input: &str) -> String {
    let no_block = strip_css_comments(input); // `/* */` share syntax
    no_block
        .lines()
        .map(|l| match l.find("//") {
            Some(i) => &l[..i],
            None => l,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn app_rs() -> String {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/src/app.rs");
    strip_rust_comments(&std::fs::read_to_string(path).expect("read src/app.rs"))
}

#[test]
fn web_only_sessions_button_is_gone() {
    assert!(
        !app_rs().contains("ocean-sessions-trigger"),
        "app.rs must not mount a host-specific Sessions text button — the Island \
         is the Sessions entry on every host (P1)",
    );
    assert!(
        !strip_css_comments(&read("styles/chrome.css")).contains(".ocean-sessions-trigger"),
        "chrome.css must not keep dead .ocean-sessions-trigger rules (P1)",
    );
}

#[test]
fn island_mounts_in_header_on_every_host() {
    let src = app_rs();
    let start = src
        .find("<header class=\"ocean-header\"")
        .expect("header exists");
    let end = start + src[start..].find("</header>").expect("header closes");
    let header = &src[start..end];
    assert!(
        header.contains("<DynamicIsland"),
        "the header must mount the Dynamic Island (P1)"
    );
    assert!(
        !header.contains("when=move || in_tauri") && !header.contains("when=move || !in_tauri"),
        "header controls must not branch on host — only the is-titlebar inset \
         and drag region are Tauri-specific (P1)",
    );
}

/// `app.rs` without the body of `should_handle_sessions_shortcut`: the one
/// documented keyboard exception (native Cmd/Ctrl+P, so web/PWA keeps browser
/// Print) is allowed to read `in_tauri`.
fn app_rs_without_shortcut_seam() -> String {
    let src = app_rs();
    let Some(at) = src.find("fn should_handle_sessions_shortcut") else {
        return src;
    };
    let open = at + src[at..].find('{').expect("shortcut helper has a body");
    let mut depth = 0usize;
    for (i, ch) in src[open..].char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    let close = open + i + 1;
                    return format!("{}{}", &src[..at], &src[close..]);
                }
            }
            _ => {}
        }
    }
    panic!("unbalanced braces in should_handle_sessions_shortcut");
}

#[test]
fn host_detection_never_gates_layout_or_panels() {
    let src = app_rs_without_shortcut_seam();
    for forbidden in ["in_tauri &&", "&& in_tauri", "!in_tauri", "DeckPanel"] {
        assert!(
            !src.contains(forbidden),
            "app.rs must not gate layout/panels on host (`{forbidden}` found) — \
             WorkspacePane is the one Files/Repo/Browser surface everywhere (P1)",
        );
    }
}
