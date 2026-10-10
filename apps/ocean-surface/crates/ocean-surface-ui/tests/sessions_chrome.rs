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

/// Exempt only the two documented native keyboard helpers, not layout code.
fn app_rs_without_shortcut_seam() -> String {
    let mut src = app_rs();
    for name in [
        "should_handle_sessions_shortcut",
        "should_handle_recall_shortcut",
    ] {
        let at = src.find(&format!("fn {name}")).expect("shortcut helper");
        let open = at + src[at..].find('{').unwrap();
        let mut depth = 0;
        let mut end = None;
        for (i, ch) in src[open..].char_indices() {
            match ch {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        end = Some(open + i + 1);
                        break;
                    }
                }
                _ => {}
            }
        }
        src.replace_range(at..end.expect("balanced helper"), "");
    }
    src
}

#[test]
fn native_shortcut_hints_match_their_bindings() {
    let src = app_rs();
    assert!(src.contains("hint: in_tauri.then(|| \"⌘P\".into())"));
    assert!(src.contains("hint: in_tauri.then(|| \"⌘⇧F\".into())"));
    let island = read("crates/ocean-surface-ui/src/island_dynamic.rs");
    for hint in ["⌘P", "⌘⇧F"] {
        let at = island
            .find(&format!(
                "<span class=\"island-stage__hint\">\"{hint}\"</span>"
            ))
            .unwrap();
        assert!(island[..at]
            .trim_end()
            .ends_with("<Show when=move || in_tauri>"));
    }
}

#[test]
fn pinned_vertical_rail_requires_closed_workspace() {
    let css = strip_css_comments(&read("styles/panels.css"));
    let wide = css.split("@media (min-width: 1480px)").nth(1).unwrap();
    assert!(wide
        .trim_start()
        .starts_with("{\n  .ocean-surface:not(.has-workspace-open) .pinned-rail"));
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
