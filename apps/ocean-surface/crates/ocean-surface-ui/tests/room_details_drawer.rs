//! Rooms — the focus-first room details drawer (design
//! `docs/OCEAN_ROOMS_DESIGN_DIRECTION.md` §5; ported from
//! Risingtides-dev/ocean-surface#240 onto the split Rooms module).
//!
//! Source/CSS guards (same style as rooms_agent_badge.rs /
//! room_attachments_contract.rs) pinning what the design settled:
//!   1. There is no permanent right rail: `.rooms-workspace__right` is one
//!      fixed slide-over at every width, closed by default, `--bg-raised` on
//!      `--shadow-lg` with no border, behind a `--scrim` backdrop, with a
//!      280ms slide whose reduced-motion kill follows the state selectors; a
//!      full-screen sheet at or below 650px; the thread panel alone docks
//!      inline at or above 1280px (§7). The 901-1080 rail gap is gone.
//!   2. The header carries the members pill (Groups glyph + count) that
//!      toggles the drawer, and the scrim closes it like Escape.
//!   3. The drawer's Members mode hosts the roster and the default-folder row
//!      (§5.2), whose Bind / Unbind are the ONLY callers of the daemon's
//!      `PATCH /v1/rooms/persistent/{key}` on this surface, sending
//!      `workspace_root` as a string (bind) or an explicit `null` (unbind).
//!   4. Every class the drawer's new markup emits has a stylesheet rule, and
//!      those rules use tokens only.

use std::path::Path;

fn read(rel: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

/// Whitespace-free view so multi-line `view!` markup matches as one string.
fn squash(input: &str) -> String {
    input.chars().filter(|c| !c.is_whitespace()).collect()
}

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

/// The production half of a module (above its `#[cfg(test)] mod tests`).
fn production(rel: &str) -> String {
    let source = read(rel);
    source
        .rsplit_once("#[cfg(test)]\nmod tests {")
        .map_or(source.clone(), |(production, _)| production.to_string())
}

const WORKSPACE: &str = "src/rooms_workspace/mod.rs";
const BINDING: &str = "src/rooms_workspace/workspace_binding.rs";
const PICKER: &str = "src/rooms_workspace/working_directory.rs";
const ROOMS: &str = "src/rooms.rs";
const OVERFLOW: &str = "src/room_overflow.rs";
const CSS: &str = "../../styles/rooms-workspace.css";

fn rooms_css() -> String {
    strip_css_comments(&read(CSS))
}

/// Brace-matched bodies of every media rule with `prelude`.
fn media_blocks(css: &str, prelude: &str) -> Vec<String> {
    let bytes = css.as_bytes();
    let mut blocks = Vec::new();
    let mut from = 0usize;
    while let Some(rel) = css[from..].find(prelude) {
        let at = from + rel;
        let Some(open_rel) = css[at..].find('{') else {
            break;
        };
        let open = at + open_rel;
        let mut depth = 0usize;
        let mut end = None;
        for (i, b) in bytes.iter().enumerate().skip(open) {
            match b {
                b'{' => depth += 1,
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        end = Some(i);
                        break;
                    }
                }
                _ => {}
            }
        }
        let Some(end) = end else { break };
        blocks.push(css[open + 1..end].to_string());
        from = end + 1;
    }
    blocks
}

/// Every `class="rooms-workspace__…"` / `class:rooms-workspace__…=` and
/// `working-directory__…` literal a module emits.
fn emitted_classes(source: &str) -> Vec<String> {
    let mut found = Vec::new();
    for prefix in [
        "rooms-workspace__",
        "working-directory__",
        "rooms-workspace__right-",
    ] {
        let mut rest = source;
        while let Some(at) = rest.find(prefix) {
            let tail = &rest[at..];
            let end = tail
                .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '-'))
                .unwrap_or(tail.len());
            let class = tail[..end].trim_end_matches('-').to_string();
            if !found.contains(&class) {
                found.push(class);
            }
            rest = &rest[at + end.max(1)..];
        }
    }
    found
}

fn css_defines_class(css: &str, class: &str) -> bool {
    let needle = format!(".{class}");
    css.match_indices(&needle).any(|(at, _)| {
        let after = css[at + needle.len()..].chars().next();
        !after.is_some_and(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    })
}

// ---- 1. One slide-over at every width -----------------------------------

#[test]
fn the_drawer_is_a_fixed_slide_over_with_elevation_and_no_border() {
    let css = squash(&rooms_css());
    let drawer = css
        .match_indices(".rooms-workspace__right{")
        .map(|(at, _)| &css[at..at + css[at..].find('}').unwrap()])
        .find(|body| body.contains("position:fixed;"))
        .expect("the canonical drawer rule fixes the drawer");
    for property in [
        "top:0;",
        "right:0;",
        "bottom:0;",
        "width:min(380px,100vw);",
        "z-index:420;",
        "border-left:none;",
        "background:var(--bg-raised);",
        "box-shadow:var(--shadow-lg);",
        "transform:translateX(100%);",
        "visibility:hidden;",
        "transition:transform280msvar(--ease),visibility0slinear280ms;",
    ] {
        assert!(
            drawer.contains(property),
            "the drawer rule must carry `{property}` (design §5): {drawer}"
        );
    }
    assert!(
        css.contains(".rooms-workspace__right--open{transform:none;visibility:visible;"),
        "`--open` slides the drawer in"
    );
    let scrim = css
        .find(".rooms-workspace__right-scrim{")
        .map(|at| &css[at..at + css[at..].find('}').unwrap()])
        .expect("scrim rule");
    assert!(
        scrim.contains("position:fixed;inset:0;") && scrim.contains("background:var(--scrim);"),
        "the scrim is the `--scrim` token over the whole viewport: {scrim}"
    );
    // The scrim sits under the drawer and both sit above the compact left
    // drawer (400) and the mobile nav (410), below the close dialog (440).
    assert!(scrim.contains("z-index:415;"), "{scrim}");
}

#[test]
fn the_reduced_motion_kill_follows_the_drawer_state_selectors() {
    let css = squash(&rooms_css());
    let open_rule = css
        .find(".rooms-workspace__right--open{transform:none;")
        .expect("open rule");
    let kill = css
        .find("@media(prefers-reduced-motion:reduce){.rooms-workspace__right,.rooms-workspace__right--open{transition:none;}}")
        .expect("a reduced-motion kill naming both drawer selectors");
    assert!(
        kill > open_rule,
        "the reduced-motion kill must come AFTER the state selectors so it wins the cascade"
    );
}

#[test]
fn the_drawer_is_full_screen_at_650_and_only_the_thread_docks_inline_at_1280() {
    let css = rooms_css();
    let compact = media_blocks(&css, "@media (max-width: 650px)");
    assert!(
        compact
            .iter()
            .any(|body| squash(body).contains(".rooms-workspace__right{width:100vw;}")),
        "at or below 650px the drawer is a full-screen sheet"
    );
    let wide = media_blocks(&css, "@media (min-width: 1280px)");
    let docked = wide
        .iter()
        .map(|body| squash(body))
        .find(|body| body.contains(".rooms-workspace__right--open.rooms-workspace__right--thread{"))
        .expect("a 1280px block docking the thread panel");
    assert!(
        docked.contains("position:static;")
            && docked.contains("flex:0 0 360px;".replace(' ', "").as_str()),
        "the docked thread panel is a 360px inline column (design §7): {docked}"
    );
    assert!(
        docked.contains(
            ".rooms-workspace__right--thread+.rooms-workspace__right-scrim{display:none;}"
        ),
        "the scrim stands down while the thread is docked"
    );
    // The members/files/search modes never dock: no selector docks `--open`
    // alone at any width, and the old rail breakpoints are gone.
    assert!(
        !css.contains("(max-width: 1080px)"),
        "the 901-1080 rail gap is gone"
    );
    assert!(!css.contains("(min-width: 901px)"));
    assert!(
        !squash(&css).contains(".rooms-workspace__right--open{display:flex;position:absolute;"),
        "no absolute-positioned rail overlay survives"
    );
}

// ---- 2. Members pill and scrim ------------------------------------------

#[test]
fn the_header_members_pill_toggles_the_drawer_and_the_scrim_closes_it() {
    let workspace = squash(&production(WORKSPACE));
    let pill_at = workspace
        .find(r#"class="rooms-workspace__members-chip""#)
        .expect("the header emits the members pill");
    let pill = &workspace[pill_at..pill_at + workspace[pill_at..].find("</button>").unwrap()];
    assert!(
        pill.contains(r#"aria-controls="rooms-workspace-details""#),
        "{pill}"
    );
    assert!(pill.contains("aria-expanded=move||members_open.get().to_string()"));
    assert!(
        pill.contains("<crate::icons::Groups/>"),
        "Groups glyph, never an emoji"
    );
    assert!(pill.contains("rooms-workspace__members-chip-count"));
    assert!(
        pill.contains("ifmembers_open.get_untracked(){close_right();}else{"),
        "the pill toggles: open members, or close the drawer"
    );
    assert!(
        pill.contains("remember_right_focus(ev.current_target());"),
        "opening remembers the pill for Escape's return focus"
    );
    assert!(
        workspace.contains(r#"id="rooms-workspace-details"class="rooms-workspace__right""#),
        "the drawer carries the id the pill controls"
    );
    assert!(
        workspace.contains(
            "class:rooms-workspace__right--thread=move||selected_thread_root_seq.get().is_some()"
        ),
        "the thread modifier lets the stylesheet dock the thread panel"
    );
    let scrim_at = workspace
        .find(r#"class="rooms-workspace__right-scrim""#)
        .expect("the scrim is emitted");
    let scrim = &workspace[scrim_at..scrim_at + workspace[scrim_at..].find("></div>").unwrap()];
    assert!(scrim.contains("on:click=move|_|close_right()"), "{scrim}");
    assert!(
        workspace.contains(
            "{move||right_open().then(||view!{<divclass=\"rooms-workspace__right-scrim\""
        ),
        "the scrim renders only while the drawer is open"
    );
    // The scrim follows the drawer in the DOM so `--thread + scrim` can hide it.
    let drawer_at = workspace.find(r#"id="rooms-workspace-details""#).unwrap();
    assert!(drawer_at < scrim_at);
    // Escape still closes exactly the drawer first.
    assert!(workspace.contains("ifright_rail_escape(right_open(),ev.default_prevented(),){ev.prevent_default();close_right();return;}"));
    // The overflow keeps its Members and Files doors.
    let overflow = read(OVERFLOW);
    assert!(overflow.contains("\"Members\"") && overflow.contains("\"Files\""));
}

#[test]
fn the_members_pill_counts_what_the_members_section_lists() {
    let workspace = squash(&production(WORKSPACE));
    assert!(
        workspace.contains("fnmembers_pill_count(room:Option<&crate::rooms::Room>,access:Option<&crate::rooms::RoomAccessProjection>,)->Option<usize>{"),
        "one pure count from the two roster sources"
    );
    assert!(workspace.contains("RoomAccessState::Local=>{room.map(|room|room.participants.len())}"));
    assert!(workspace.contains("Some(access)=>Some(access.members.len()),"));
    assert!(workspace.contains("None=>None,"));
}

// ---- 3. Members mode: roster + default folder, the PATCH callers ---------

#[test]
fn the_drawer_members_mode_mounts_the_roster_then_the_default_folder_row() {
    let workspace = squash(&production(WORKSPACE));
    let title_at = workspace
        .find("}else{\"Roomdetails\"}}")
        .expect("the drawer titles its Members mode \"Room details\"");
    assert!(workspace.contains("\"Closeroomdetails\""));
    let roster_at = workspace
        .find("<divclass=\"rooms-workspace__right-section-title\">\"Members\"</div><MembersPanelrooms=rooms/><DefaultFolderSectionrooms=roomsprojects=projects/>")
        .expect("Members section, roster, then the default-folder row");
    assert!(title_at < roster_at);
    // The folder row reads the same shared catalogue the create form reads.
    assert!(workspace
        .contains("letprojects:Signal<Vec<crate::daemon::ProjectInfo>>=daemon.projects.into();"));
}

#[test]
fn bind_and_unbind_are_the_only_patch_callers_and_send_string_or_explicit_null() {
    let binding = squash(&production(BINDING));
    assert!(
        binding.contains("rooms.set_open_room_workspace(Some(root));"),
        "Bind sends the path"
    );
    assert!(
        binding.contains("rooms.set_open_room_workspace(None);"),
        "Unbind sends None"
    );
    assert!(binding.contains(r#"class="rooms-workspace__workspace-bind""#));
    assert!(binding.contains(r#"class="rooms-workspace__workspace-unbind""#));
    assert!(binding.contains("\"Bind\"") && binding.contains("\"Unbind\""));
    // Design §5.2: the row shows the root (or "none") in mono with a ghost
    // Edit; the controls are revealed, not permanent.
    assert!(binding.contains(r#"class="rooms-workspace__workspace-root""#));
    assert!(binding.contains("Some(root)=>root.to_string(),None=>\"none\".to_string(),"));
    assert!(binding.contains(r#"class="rooms-workspace__workspace-edit""#));
    assert!(binding.contains("<Showwhen=move||editing.get()&&can_edit()>"));
    // Absent, not disabled, when this host may not write the room's local store.
    assert!(
        binding.contains("letcan_edit=move||local_store_write_gate(rooms.access.get().as_ref());")
    );
    assert!(binding.contains(
        "{move||can_edit().then(||view!{<buttonclass=\"rooms-workspace__workspace-edit\""
    ));

    // No other production module calls the PATCH dispatcher.
    let src_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut callers = Vec::new();
    let mut stack = vec![src_dir];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).expect("read_dir") {
            let path = entry.expect("entry").path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
                let source = std::fs::read_to_string(&path).unwrap_or_default();
                let production = source
                    .rsplit_once("#[cfg(test)]\nmod tests {")
                    .map_or(source.as_str(), |(p, _)| p);
                if production.contains("set_open_room_workspace(") {
                    callers.push(path.file_name().unwrap().to_string_lossy().to_string());
                }
            }
        }
    }
    callers.sort();
    assert_eq!(
        callers,
        vec!["rooms.rs".to_string(), "workspace_binding.rs".to_string()],
        "only the default-folder row may bind or unbind"
    );

    // The dispatcher sends PATCH to the room route with the one-field body,
    // whose `None` is an explicit null (no skip_serializing_if).
    let rooms = production(ROOMS);
    let dispatcher_at = rooms
        .find("pub fn set_open_room_workspace(&self, workspace_root: Option<String>)")
        .expect("the PATCH dispatcher");
    let dispatcher =
        &rooms[dispatcher_at..dispatcher_at + rooms[dispatcher_at..].find("\n    }\n").unwrap()];
    assert!(dispatcher.contains("format!(\"{base}/v1/rooms/persistent/{}\", encode(&key))"));
    assert!(dispatcher.contains("Request::patch(&url)"));
    assert!(dispatcher.contains("RoomWorkspacePatchBody::bind(&root)"));
    assert!(dispatcher.contains("RoomWorkspacePatchBody::unbind()"));
    assert!(
        dispatcher.contains("if !me.room_is_current(generation_id, &key) {"),
        "replies land only in their admission"
    );
    let body_at = rooms
        .find("pub(crate) struct RoomWorkspacePatchBody {")
        .expect("the PATCH body type");
    let body = &rooms[body_at..body_at + rooms[body_at..].find('}').unwrap()];
    assert!(body.contains("pub(crate) workspace_root: Option<String>,"));
    assert!(
        !body.contains("skip_serializing_if"),
        "an unbind must serialize `\"workspace_root\": null`; an absent field means unchanged"
    );
    // No operator key, no header beyond the content type: the route is
    // loopback-trust, forwarded generically by the proxy.
    assert!(
        !dispatcher.contains("X-Ocean-Operator") && !dispatcher.contains("room_owner_mutation")
    );
}

// ---- 4. CSS ↔ emitter parity, tokens only --------------------------------

#[test]
fn every_class_the_drawer_markup_emits_has_a_rule_and_the_new_rules_use_tokens_only() {
    let css = rooms_css();
    // The two new modules are styled wholly by rooms-workspace.css.
    for module in [BINDING, PICKER] {
        for class in emitted_classes(&production(module)) {
            assert!(
                css_defines_class(&css, &class),
                "{module} emits `.{class}`, which styles/rooms-workspace.css does not define"
            );
        }
    }
    // The workspace shares classes with rooms-attention.css and others; pin
    // the drawer's own new markup here.
    let workspace = production(WORKSPACE);
    for class in [
        "rooms-workspace__members-chip",
        "rooms-workspace__members-chip-count",
        "rooms-workspace__right-scrim",
        "rooms-workspace__right-section-title",
        "rooms-workspace__right--thread",
    ] {
        assert!(workspace.contains(class), "{WORKSPACE} must emit `{class}`");
        assert!(
            css_defines_class(&css, class),
            "{WORKSPACE} emits `.{class}`, which styles/rooms-workspace.css does not define"
        );
    }
    // The drawer, pill, default-folder and picker regions: tokens only.
    let squashed = squash(&css);
    for (start, end) in [
        (
            ".rooms-workspace__right{display:flex;position:fixed;",
            ".rooms-workspace__right-list{padding:10px8px;}",
        ),
        (
            ".working-directory{",
            "@media(pointer:coarse){.working-directory__select",
        ),
    ] {
        let from = squashed
            .find(start)
            .unwrap_or_else(|| panic!("region start `{start}`"));
        let to = from
            + squashed[from..]
                .find(end)
                .unwrap_or_else(|| panic!("region end `{end}`"));
        let region = &squashed[from..to];
        assert!(
            !region.contains('#') && !region.contains("rgb(") && !region.contains("hsl("),
            "colour literals in the drawer/picker rules; colours live only in tokens.css"
        );
    }
}
