//! `docs/OCEAN_ROOMS_PRODUCT.md` may only name what exists.
//!
//! An audit found the product doc describing a surface that was never built:
//! `Local`/`Remote`/`None` access states, a `mention_ids` list on the message
//! body, the surface posting room turns through the session turn route, and a
//! `.rooms-panel__list` rule no component emits. Nothing held the doc, so it
//! drifted with every gate green. This guard holds the facts a reader acts on
//! and a scanner can check:
//!
//! * every ACCESS STATE the doc names is one of `RoomAccessState`'s variants,
//!   read from `src/rooms.rs` itself so the doc cannot agree with a list this
//!   file made up;
//! * every `.rooms-*` CSS class the doc names is defined in `styles/` AND
//!   emitted from production Rust (page or Tauri shell) — a class with a
//!   stylesheet rule and no emitter is exactly the dead-selector case;
//! * every route in the doc's "Daemon API Summary" table is one the page
//!   (`crates/ocean-surface-ui/src`) or the Tauri shell
//!   (`crates/ocean-tauri/src`) actually sends; and, in the monorepo, one the
//!   daemon's router registers with that method.
//!
//! Ported from Risingtides-dev/ocean-surface#228 against the monorepo wire:
//! this surface opens rooms off `GET /v1/rooms/persistent/{key}` (not the
//! snapshot route) and never calls `/transcript` or `/snapshot` itself, so the
//! legacy hydration claims were re-derived rather than copied.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

const DOC: &str = "docs/OCEAN_ROOMS_PRODUCT.md";

/// The five wire states. Asserted equal to the enum below, never trusted alone.
const ACCESS_STATES: [&str; 5] = ["local", "connecting", "live", "recovering", "revoked"];

/// Backticked tokens that legitimately share a line with the word "access"
/// without being an access state. Anything else backticked and bare-word on
/// such a line is read as a claimed state and must be one of the five.
const ACCESS_LINE_NON_STATES: [&str; 5] = ["access", "id", "outbox", "seq", "state"];

/// The audit's specific ghosts, independent of formatting.
const GHOSTS: [(&str, &str); 7] = [
    ("`Remote`", "there is no remote access state"),
    ("`remote`", "there is no remote access state"),
    (
        "mention_ids",
        "mentions travel as `@id` text in `body`; there is no mention list",
    ),
    (
        "rooms-panel__list",
        "the rail list is `.rooms-workspace__left-list`; `.rooms-panel__list` has no emitter",
    ),
    (
        "sends no `trigger_policy`",
        "every create carries `RoomTriggerPolicy::create_default()` (`CreateRoomBody::new`)",
    ),
    (
        "sends neither",
        "the create body always carries the canonical wake policy; the PATCH route carries workspace_root",
    ),
    (
        "this surface does not call it",
        "the drawer's Default folder row sends PATCH /v1/rooms/persistent/{key} with workspace_root",
    ),
];

/// Where the daemon's router lives relative to the Surface component in the
/// monorepo. Absent in a standalone Surface checkout, where the daemon-side
/// check is skipped (the page-side check still runs).
const DAEMON_MAIN: &str = "../../platform/ocean-os/crates/ocean-daemon/src/main.rs";

fn repo_root() -> &'static Path {
    // tests/ -> ocean-surface-ui/ -> crates/ -> Surface component root
    Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
}

fn read(rel: &str) -> String {
    let path = repo_root().join(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

/// The production half of a Rust module: everything above its trailing
/// `#[cfg(test)] mod tests`, so a needle a module's own unit tests quote does
/// not satisfy a scan. Modules without that trailer are taken whole.
fn production(source: &str) -> &str {
    source
        .rsplit_once("#[cfg(test)]\nmod tests {")
        .map_or(source, |(production, _)| production)
}

/// Every `.rs` file under `dir`, production half only, read into one blob.
fn rust_src(dir: PathBuf) -> String {
    let mut blob = String::new();
    let mut stack = vec![dir];
    while let Some(dir) = stack.pop() {
        for entry in
            std::fs::read_dir(&dir).unwrap_or_else(|e| panic!("read_dir {}: {e}", dir.display()))
        {
            let path = entry.expect("dir entry").path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
                let source = std::fs::read_to_string(&path).unwrap_or_default();
                blob.push_str(production(&source));
                blob.push('\n');
            }
        }
    }
    blob
}

/// The page: `crates/ocean-surface-ui/src`.
fn page_src() -> String {
    rust_src(Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/src")).to_path_buf())
}

/// The native shell: `crates/ocean-tauri/src` (operator-key routes the page
/// only asks for).
fn shell_src() -> String {
    rust_src(repo_root().join("crates/ocean-tauri/src"))
}

fn all_styles() -> String {
    let dir = repo_root().join("styles");
    let mut blob = String::new();
    for entry in std::fs::read_dir(&dir).expect("styles/ exists") {
        let path = entry.expect("styles entry").path();
        if path.extension().and_then(|e| e.to_str()) == Some("css") {
            blob.push_str(&std::fs::read_to_string(&path).unwrap_or_default());
            blob.push('\n');
        }
    }
    blob
}

/// The variants of `pub enum RoomAccessState { .. }` in `src/rooms.rs`,
/// snake-cased the way `#[serde(rename_all = "snake_case")]` puts them on the
/// wire (all five are single words, so lowercasing is that conversion).
fn access_state_variants() -> BTreeSet<String> {
    let rooms = read("crates/ocean-surface-ui/src/rooms.rs");
    let body = rooms
        .split_once("pub enum RoomAccessState {")
        .expect("src/rooms.rs declares `pub enum RoomAccessState`")
        .1
        .split_once('}')
        .expect("RoomAccessState's body closes")
        .0;
    body.split(',')
        .map(|variant| variant.trim().to_lowercase())
        .filter(|variant| !variant.is_empty())
        .collect()
}

/// Every `` `token` `` on `line` shaped like a state name: one alphabetic word,
/// either the wire form (`live`) or a bare variant (`Live`). Multi-capital
/// type names such as `RoomAccessProjection`, paths, and snake_case
/// identifiers are not state names and are skipped.
fn backticked_words(line: &str) -> Vec<&str> {
    line.split('`')
        .skip(1)
        .step_by(2)
        .filter(|token| {
            let mut chars = token.chars();
            chars
                .next()
                .is_some_and(|first| first.is_ascii_alphabetic())
                && chars.all(|c| c.is_ascii_lowercase())
        })
        .collect()
}

/// The body of the doc's `### Access States` section, up to the next heading.
fn access_states_section(doc: &str) -> &str {
    let after = doc
        .split_once("### Access States")
        .expect("the product doc keeps an `### Access States` section")
        .1;
    match after.find("\n#") {
        Some(end) => &after[..end],
        None => after,
    }
}

fn is_class_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '-'
}

/// Every `.rooms-<class>` the doc names, dot stripped.
fn doc_rooms_selectors(doc: &str) -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    let mut rest = doc;
    while let Some(offset) = rest.find(".rooms-") {
        let start = offset + 1;
        let end = rest[start..]
            .find(|c: char| !is_class_char(c))
            .map_or(rest.len(), |len| start + len);
        found.insert(rest[start..end].trim_end_matches('-').to_string());
        rest = &rest[end..];
    }
    found
}

/// `needle` occurs in `hay` as a whole class name — not merely as the prefix
/// or suffix of a longer live class, which would let `.rooms-workspace__left`
/// ride on `.rooms-workspace__left-list`.
fn contains_whole_class(hay: &str, needle: &str) -> bool {
    hay.match_indices(needle).any(|(at, _)| {
        let before = hay[..at].chars().next_back();
        let after = hay[at + needle.len()..].chars().next();
        !before.is_some_and(is_class_char) && !after.is_some_and(is_class_char)
    })
}

/// `{key}`, `{id}`, `{agent_member_id}` … all become `{}`: the page formats
/// its paths with `{}` placeholders, the daemon registers them with named
/// ones, and the doc writes whichever reads best.
fn normalize_path(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    let mut rest = path;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        out.push_str("{}");
        rest = match rest[open..].find('}') {
            Some(close) => &rest[open + close + 1..],
            None => "",
        };
    }
    out.push_str(rest);
    out
}

/// The "Daemon API Summary" table: `(methods, normalized path)` per row, the
/// query string dropped (`/events?after_seq=` names the route `/events`).
fn doc_api_rows(doc: &str) -> Vec<(Vec<String>, String)> {
    let table = doc
        .split_once("## Daemon API Summary")
        .expect("the product doc keeps a `## Daemon API Summary` table")
        .1;
    table
        .lines()
        .filter_map(|line| {
            let cells: Vec<&str> = line.split('|').map(str::trim).collect();
            let (method_cell, path_cell) = (cells.get(1)?, cells.get(2)?);
            let path = path_cell.strip_prefix("`/")?.strip_suffix('`')?;
            let path = format!("/{}", path.split('?').next().unwrap_or_default());
            let methods = method_cell
                .split('/')
                .map(|m| m.trim().trim_matches('`').to_string())
                .filter(|m| !m.is_empty())
                .collect();
            Some((methods, normalize_path(&path)))
        })
        .collect()
}

/// `path` is sent as a whole route by `blob`: the literal is followed by the
/// end of its string or its query (`"{base}/v1/rooms/persistent/{}/prefs"`,
/// `"{base}/v1/rooms/inbox?limit=50"`). The Tauri shell builds room routes as
/// `format!("{}/agents/bootstrap", room(r))`, so a `/v1/rooms/persistent/{}`
/// route may also appear as `{}` + its suffix.
fn blob_sends_path(blob: &str, path: &str) -> bool {
    let whole = blob
        .match_indices(path)
        .any(|(at, _)| matches!(blob[at + path.len()..].chars().next(), Some('"' | '?')));
    if whole {
        return true;
    }
    path.strip_prefix("/v1/rooms/persistent/{}")
        .filter(|suffix| !suffix.is_empty())
        .is_some_and(|suffix| blob.contains(&format!("{{}}{suffix}\"")))
}

/// The daemon's registered routes, normalized path → methods, parsed from
/// every `.route("<path>", <handlers>)` in `ocean-daemon/src/main.rs`. `None`
/// outside the monorepo.
fn daemon_routes() -> Option<BTreeMap<String, BTreeSet<String>>> {
    let main = std::fs::read_to_string(repo_root().join(DAEMON_MAIN)).ok()?;
    let mut routes: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut rest = main.as_str();
    while let Some(at) = rest.find(".route(") {
        rest = rest[at + ".route(".len()..].trim_start();
        let Some(path) = rest.strip_prefix('"') else {
            continue;
        };
        let Some((path, after)) = path.split_once('"') else {
            break;
        };
        // The handler expression runs to the `)` that closes this `.route(`.
        let mut depth = 1usize;
        let mut end = after.len();
        for (i, c) in after.char_indices() {
            match c {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        end = i;
                        break;
                    }
                }
                _ => {}
            }
        }
        let handlers = &after[..end];
        let methods = routes.entry(normalize_path(path)).or_default();
        for method in ["get", "post", "put", "patch", "delete"] {
            if handlers
                .match_indices(&format!("{method}("))
                .any(|(at, _)| {
                    !handlers[..at]
                        .chars()
                        .next_back()
                        .is_some_and(|c| c.is_ascii_alphanumeric() || c == '_')
                })
            {
                methods.insert(method.to_uppercase());
            }
        }
        rest = &after[end..];
    }
    Some(routes)
}

/// The wake policy `CreateRoomBody::new` attaches to every create, as the
/// daemon receives it: every flag present, `on_schedule` absent. Asserted
/// against production `rooms.rs` below, never trusted alone.
fn canonical_wake_policy() -> serde_json::Value {
    serde_json::json!({
        "on_mention": true,
        "on_thread_reply": true,
        "on_component_event": false,
        "on_build_failure": false,
        "on_ci_failure": false,
    })
}

/// The JSON body of the doc's "### 1. Creating a Room" request: the first
/// fenced block after that heading, minus its `POST …` request line and `->`
/// response lines.
fn doc_create_body(doc: &str) -> serde_json::Value {
    let section = doc
        .split_once("### 1. Creating a Room")
        .expect("the product doc keeps a `### 1. Creating a Room` section")
        .1;
    let block = section
        .split_once("```\n")
        .expect("the create section opens a fenced block")
        .1
        .split_once("\n```")
        .expect("the create section's fenced block closes")
        .0;
    let body = block
        .lines()
        .filter(|line| !line.starts_with("POST ") && !line.starts_with("->"))
        .collect::<Vec<_>>()
        .join("\n");
    serde_json::from_str(&body)
        .unwrap_or_else(|e| panic!("{DOC}'s create body is not the JSON the surface sends: {e}"))
}

/// Surface-created rooms carry the canonical wake policy. ocean-core never
/// convenes on an absent policy ("no trigger policy configured"), so the
/// `None` the rail once passed made every room created here invoke-only while
/// the Ocean MCP instructions, this doc and the TUI all said "mention an agent
/// with @name to wake it". The default is `RoomTriggerPolicy::create_default`
/// (the "+ New room" policy in `OCEAN_ROOMS_DESIGN_DIRECTION.md` and the Gate 1
/// create flow), attached by `CreateRoomBody::new` so no caller can drop it;
/// the doc's create body must show exactly that object.
#[test]
fn product_doc_create_body_carries_the_wake_policy_rooms_rs_sends() {
    let rooms: String = production(&read("crates/ocean-surface-ui/src/rooms.rs"))
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    assert!(
        rooms.contains(
            "pubfncreate_default()->Self{Self{on_mention:true,on_thread_reply:true,\
             on_component_event:false,on_build_failure:false,on_ci_failure:false,\
             on_schedule:None,}}"
        ),
        "RoomTriggerPolicy::create_default in src/rooms.rs is no longer on_mention + \
         on_thread_reply with everything else off; update {DOC}, canonical_wake_policy() \
         and this needle together",
    );
    assert!(
        rooms.contains("trigger_policy:RoomTriggerPolicy::create_default(),"),
        "CreateRoomBody::new must attach RoomTriggerPolicy::create_default(); a create \
         without a policy is invoke-only",
    );

    let doc = read(DOC);
    assert_eq!(
        doc_create_body(&doc)["trigger_policy"],
        canonical_wake_policy(),
        "{DOC}'s create body must show exactly the trigger_policy CreateRoomBody::new sends",
    );
    assert!(
        doc.contains("`RoomTriggerPolicy::create_default()`"),
        "{DOC} must name RoomTriggerPolicy::create_default() as the policy every create carries",
    );
}

#[test]
fn the_five_access_states_are_the_enum_s_variants() {
    let expected: BTreeSet<String> = ACCESS_STATES.iter().map(|s| s.to_string()).collect();
    assert_eq!(
        access_state_variants(),
        expected,
        "RoomAccessState in src/rooms.rs no longer has exactly the five wire states; \
         update ACCESS_STATES here and docs/OCEAN_ROOMS_PRODUCT.md together",
    );
}

#[test]
fn product_doc_lists_exactly_the_five_access_states() {
    let doc = read(DOC);
    let listed: BTreeSet<String> = access_states_section(&doc)
        .lines()
        .filter_map(|line| line.trim_start().strip_prefix("- `"))
        .filter_map(|rest| rest.split_once('`').map(|(state, _)| state.to_string()))
        .collect();
    assert_eq!(
        listed,
        access_state_variants(),
        "{DOC}'s `### Access States` list must name exactly RoomAccessState's variants \
         in wire form",
    );
}

#[test]
fn product_doc_names_no_access_state_outside_the_five() {
    let doc = read(DOC);
    let variants = access_state_variants();
    let section = access_states_section(&doc);
    let mut offenders = Vec::new();
    for (number, line) in doc.lines().enumerate() {
        let names_access = line.to_lowercase().contains("access");
        let in_state_list = line.contains("- `") && section.contains(line);
        if !names_access && !in_state_list {
            continue;
        }
        for word in backticked_words(line) {
            let lowered = word.to_lowercase();
            if !variants.contains(&lowered) && !ACCESS_LINE_NON_STATES.contains(&lowered.as_str()) {
                offenders.push(format!("{DOC}:{} names `{word}`", number + 1));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "backticked words beside room access must be one of the access states \
         {variants:?} (RoomAccessState in src/rooms.rs); a word that is not a state \
         goes in ACCESS_LINE_NON_STATES:\n{}",
        offenders.join("\n"),
    );
    for (ghost, why) in GHOSTS {
        assert!(!doc.contains(ghost), "{DOC} names {ghost}: {why}");
    }
}

#[test]
fn product_doc_names_only_rendered_rooms_selectors() {
    let doc = read(DOC);
    let selectors = doc_rooms_selectors(&doc);
    assert!(
        !selectors.is_empty(),
        "{DOC} names no `.rooms-*` selector — the scan found nothing to hold, so \
         either the doc lost its rail/transcript classes or this scanner broke",
    );
    let styles = all_styles();
    let rust = format!("{}\n{}", page_src(), shell_src());
    for class in &selectors {
        assert!(
            contains_whole_class(&styles, &format!(".{class}")),
            "{DOC} names `.{class}`, which no stylesheet in styles/ defines",
        );
        assert!(
            contains_whole_class(&rust, class),
            "{DOC} names `.{class}`, which no production Rust source emits — a dead \
             selector (the doc once bound `.rooms-panel__list`, styled but never rendered)",
        );
    }
}

#[test]
fn product_doc_api_table_names_only_routes_the_surface_sends() {
    let doc = read(DOC);
    let rows = doc_api_rows(&doc);
    assert!(
        rows.len() >= 20,
        "{DOC}'s API table parsed to {} rows; the table or this parser broke",
        rows.len(),
    );
    let page = page_src();
    let shell = shell_src();
    let mut missing = Vec::new();
    for (_, path) in &rows {
        if !blob_sends_path(&page, path) && !blob_sends_path(&shell, path) {
            missing.push(path.clone());
        }
    }
    assert!(
        missing.is_empty(),
        "{DOC}'s API table lists routes neither crates/ocean-surface-ui/src nor \
         crates/ocean-tauri/src sends (whole path, `{{}}` placeholders):\n{}",
        missing.join("\n"),
    );
}

#[test]
fn product_doc_api_table_names_only_routes_the_daemon_serves() {
    let Some(routes) = daemon_routes() else {
        eprintln!("{DAEMON_MAIN} is not beside this checkout; daemon-side route check skipped");
        return;
    };
    assert!(
        routes.contains_key("/v1/rooms/persistent/{}/events"),
        "the daemon router parse found no room events route; the parser broke",
    );
    let doc = read(DOC);
    let mut wrong = Vec::new();
    for (methods, path) in doc_api_rows(&doc) {
        match routes.get(&path) {
            None => wrong.push(format!("{path}: not registered by the daemon")),
            Some(served) => {
                for method in methods {
                    if !served.contains(&method) {
                        wrong.push(format!("{method} {path}: the daemon serves {served:?}"));
                    }
                }
            }
        }
    }
    assert!(
        wrong.is_empty(),
        "{DOC}'s API table disagrees with ocean-daemon's router:\n{}",
        wrong.join("\n"),
    );
}
