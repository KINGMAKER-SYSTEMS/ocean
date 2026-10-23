//! Rooms parity — the "agent" badge on transcript message rows.
//!
//! Source/CSS guard (same style as open_transcript_layout.rs /
//! dead_selector_removal.rs) pinning the author-kind marker so it cannot
//! drift between the three row shapes or grow its own palette:
//!   1. Every transcript author line — the timeline root in
//!      rooms_workspace/mod.rs, the thread root and the thread reply in
//!      rooms_workspace/threads.rs — emits `.rooms-workspace__msg-kind`
//!      through `author_kind_badge(..)`, between the name and the time.
//!   2. The class is painted by the SAME rule as the mention suggestion's
//!      `.rooms-workspace__mention-kind` in styles/rooms-workspace.css (one
//!      kind badge, no second palette), and that rule uses tokens only.
//!   3. Grouped rows (name sr-only, time parked in the gutter) hide the badge.
//!   4. The badge is agent-only and never on compact system rows:
//!      `author_kind_badge` compares against `RoomParticipantKind::Agent` and
//!      no other variant.

use std::path::Path;

fn read(rel: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

/// Whitespace-free view so multi-line `view!` markup matches as one string.
fn squash(input: &str) -> String {
    input.chars().filter(|c| !c.is_whitespace()).collect()
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

fn rooms_css() -> String {
    squash(&strip_css_comments(&read(
        "../../styles/rooms-workspace.css",
    )))
}

/// The declaration block of the rule whose squashed selector is EXACTLY
/// `selector`: it starts a rule (file start or right after a `}`) and is
/// immediately followed by `{`, so `.a,.b{` never answers for `.b`.
fn rule_body(css: &str, selector: &str) -> String {
    let needle = format!("{selector}{{");
    let mut from = 0;
    while let Some(rel) = css[from..].find(&needle) {
        let at = from + rel;
        if at == 0 || css[..at].ends_with('}') {
            let body = &css[at + needle.len()..];
            return body[..body.find('}').expect("rule closes")].to_string();
        }
        from = at + 1;
    }
    panic!("no rule with exact selector `{selector}` in rooms-workspace.css");
}

/// Every author line of `src`: the squashed segment from one
/// `class="rooms-workspace__msg-author"` to the `<time` that follows it.
fn author_lines(src: &str) -> Vec<&str> {
    let open = r#"class="rooms-workspace__msg-author""#;
    let mut lines = Vec::new();
    let mut from = 0;
    while let Some(rel) = src[from..].find(open) {
        let at = from + rel;
        let end = at
            + src[at..]
                .find("<time")
                .expect("author line carries the time");
        lines.push(&src[at..end]);
        from = end;
    }
    lines
}

// ---- 1. Every author line emits the badge between name and time ----------

#[test]
fn every_transcript_author_line_emits_the_agent_badge_between_name_and_time() {
    let sites = [
        ("src/rooms_workspace/mod.rs", 1usize),
        ("src/rooms_workspace/threads.rs", 2usize),
    ];
    for (rel, expected) in sites {
        let src = squash(&read(rel));
        let lines = author_lines(&src);
        assert_eq!(
            lines.len(),
            expected,
            "{rel}: expected {expected} transcript author line(s); the row shapes moved"
        );
        for line in lines {
            let name = line
                .find(r#"class="rooms-workspace__msg-name""#)
                .unwrap_or_else(|| panic!("{rel}: author line without the name span:\n{line}"));
            let badge = line.find("author_kind_badge(").unwrap_or_else(|| {
                panic!("{rel}: author line without `author_kind_badge(`:\n{line}")
            });
            assert!(
                line.contains(r#"<spanclass="rooms-workspace__msg-kind">{label}</span>"#),
                "{rel}: the badge must be the `.rooms-workspace__msg-kind` span:\n{line}"
            );
            assert!(
                name < badge,
                "{rel}: the badge follows the name (and precedes the time):\n{line}"
            );
        }
    }
}

// ---- 2. Shared rule with the mention kind label; tokens only -------------

#[test]
fn badge_shares_the_mention_kind_rule_and_paints_with_tokens_only() {
    let css = rooms_css();
    let shared = rule_body(
        &css,
        ".rooms-workspace__mention-kind,.rooms-workspace__msg-kind",
    );
    for decl in [
        "color:var(--fg-4)",
        "border:1pxsolidvar(--border-subtle)",
        "text-transform:uppercase",
        "font-size:10px",
    ] {
        assert!(
            shared.contains(decl),
            "the shared kind-badge rule must keep `{decl}`; body was:\n{shared}"
        );
    }
    for literal in ["#", "rgb(", "rgba(", "hsl(", "oklch("] {
        assert!(
            !shared.contains(literal),
            "the kind badge paints with tokens only; found `{literal}` in:\n{shared}"
        );
    }
    // The badge's own rule is layout only: no second palette for the transcript.
    let own = rule_body(&css, ".rooms-workspace__msg-kind");
    for painted in ["color", "background", "border"] {
        assert!(
            !own.contains(painted),
            "`.rooms-workspace__msg-kind` must not repaint (`{painted}`); it inherits the \
             shared kind-badge rule. Body was:\n{own}"
        );
    }
    // Exactly one definition of the mention label, and it is the shared one.
    assert_eq!(
        css.matches(".rooms-workspace__mention-kind{").count(),
        1,
        "the mention label keeps one standalone rule (its `margin-left:auto`)"
    );
    assert!(rule_body(&css, ".rooms-workspace__mention-kind").contains("margin-left:auto"));
}

// ---- 3. Grouped rows hide the badge --------------------------------------

#[test]
fn grouped_rows_hide_the_badge_with_the_name() {
    let css = rooms_css();
    let grouped = rule_body(
        &css,
        ".rooms-workspace__msg--grouped.rooms-workspace__msg-kind",
    );
    assert!(
        grouped.contains("display:none"),
        "grouped rows park the time in the gutter and hide the name, so the badge \
         must hide too; body was:\n{grouped}"
    );
}

// ---- 4. Agent-only, never on system rows ---------------------------------

#[test]
fn badge_is_agent_only_and_skips_compact_system_rows() {
    let src = squash(&read("src/rooms_workspace/format.rs"));
    let start = src.find("fnauthor_kind_badge(").expect("author_kind_badge");
    let body = &src[start..];
    let body = &body[..body.find('}').expect("fn closes")];
    assert!(
        body.contains("kind==RoomParticipantKind::Agent"),
        "the badge is gated on the Agent kind:\n{body}"
    );
    assert!(
        body.contains("!is_system"),
        "compact system rows never carry the badge"
    );
    assert!(
        body.contains("participant_kind_label(kind)"),
        "the badge word is the mention suggestion's kind label, not a second string"
    );
    for other in ["::Human", "::Bot", "::Tool", "::System"] {
        assert!(
            !body.contains(other),
            "only agents are badged; `{other}` must not appear in the gate:\n{body}"
        );
    }
    // The three call sites pass the row's own system flag, so a join/leave or
    // audit row authored by an agent stays an unlabelled compact line.
    for rel in [
        "src/rooms_workspace/mod.rs",
        "src/rooms_workspace/threads.rs",
    ] {
        let site = squash(&read(rel));
        let mut from = 0;
        while let Some(rel_at) = site[from..].find("author_kind_badge(") {
            let at = from + rel_at;
            let call = &site[at..at + site[at..].find(')').unwrap()];
            assert!(
                call.ends_with("is_system"),
                "{rel}: `{call})` must pass the row's system flag"
            );
            from = at + 1;
        }
    }
}
