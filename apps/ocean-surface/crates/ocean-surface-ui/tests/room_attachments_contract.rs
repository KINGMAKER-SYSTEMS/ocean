//! Room attachments — source and stylesheet guards for the Files feature
//! (`src/rooms_workspace/attachments.rs`, the composer's attach control, the
//! right rail's Files mode, and the `styles/rooms-workspace.css` section).
//!
//! What a unit test cannot reach is pinned by reading the source, the way
//! `rooms_product_doc_truth.rs` and `dead_selector_removal.rs` do:
//!
//! * the page sends exactly the daemon's three attachment routes, with the
//!   upload's metadata in the QUERY STRING — never multipart, a JSON body or
//!   a custom header (the daemon's CORS allows only `content-type` and
//!   `authorization`, so a custom header passes under curl and dies at the
//!   browser preflight) — and never the daemon's `DELETE`;
//! * every `rooms-workspace__file*` / `__composer-attach` class the page
//!   emits is styled, and every styled one is emitted (no dead selectors, no
//!   unstyled markup), with colors only from tokens;
//! * the attachments state and the real `<input type=file>` are mounted at
//!   `RoomsWorkspace` scope, outside the `open_room` closure, and the
//!   admission Effect retires the state with the room;
//! * both attach controls take `local_store_write_gate`, not the composer's
//!   `access_allows_writes` — which gate a control takes IS its ruling on
//!   whether its write has to reach a peer;
//! * the client-side guards mirror the daemon's constants when the daemon is
//!   beside this checkout.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

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
/// not satisfy a scan.
fn production(source: &str) -> &str {
    source
        .rsplit_once("#[cfg(test)]\nmod tests {")
        .map_or(source, |(production, _)| production)
}

fn page_module(rel: &str) -> String {
    production(&read(&format!("crates/ocean-surface-ui/src/{rel}"))).to_string()
}

/// Every `.rs` file under `src/`, production half only.
fn page_src() -> String {
    let mut blob = String::new();
    let mut stack = vec![PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/src"))];
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

fn is_class_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '-'
}

/// Every class token starting with `prefix` in `hay`, as whole class names.
fn classes_with_prefix(hay: &str, prefix: &str) -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    for (at, _) in hay.match_indices(prefix) {
        let before = hay[..at].chars().next_back();
        if before.is_some_and(is_class_char) {
            continue;
        }
        let end = hay[at..]
            .find(|c: char| !is_class_char(c))
            .map_or(hay.len(), |len| at + len);
        found.insert(hay[at..end].trim_end_matches('-').to_string());
    }
    found
}

const ATTACHMENTS: &str = "rooms_workspace/attachments.rs";
const WORKSPACE: &str = "rooms_workspace/mod.rs";
const COMPOSER: &str = "rooms_workspace/composer.rs";
const OVERFLOW: &str = "room_overflow.rs";
const CSS: &str = "styles/rooms-workspace.css";
const DAEMON_ATTACHMENTS: &str =
    "../../platform/ocean-os/crates/ocean-daemon/src/room_attachments.rs";
const PROXY_MAIN: &str = "crates/ocean-surface-proxy/src/main.rs";

/// The window of `source` from `open` to the next `/>` — one component's
/// attribute list in a `view!`.
fn attribute_window<'a>(source: &'a str, open: &str) -> &'a str {
    let start = source
        .find(open)
        .unwrap_or_else(|| panic!("{open} is not mounted"));
    let rest = &source[start..];
    let end = rest
        .find("/>")
        .unwrap_or_else(|| panic!("{open} never closes"));
    &rest[..end]
}

#[test]
fn the_page_sends_the_three_attachment_routes_with_metadata_in_the_query() {
    let module = page_module(ATTACHMENTS);
    for route in [
        // The list: the bare path, nothing else on it.
        "\"{base}/v1/rooms/persistent/{}/attachments\"",
        // The upload: filename, declared type and uploader in the QUERY.
        "\"{base}/v1/rooms/persistent/{}/attachments?filename={}&content_type={}&uploader_id={}\"",
        // The bytes, linked.
        "\"{base}/v1/rooms/persistent/{}/attachments/{}\"",
    ] {
        assert!(
            module.contains(route),
            "{ATTACHMENTS} must build the route literal {route}"
        );
    }
    // The upload is a raw body: the picked File itself. Not multipart, not
    // JSON, and never a custom header the browser preflight would refuse.
    assert!(
        module.contains("Request::post(&url).body(file)"),
        "{ATTACHMENTS} must hand the picked File to fetch as the raw body"
    );
    // Code constructs, not prose: the module header is allowed to SAY
    // "not multipart" while the code must never build one.
    for forbidden in [
        "FormData",
        "multipart/form-data",
        ".header(\"X-Ocean",
        ".header(\"x-ocean",
        ".json(&",
    ] {
        assert!(
            !module.contains(forbidden),
            "{ATTACHMENTS} must not carry upload metadata as {forbidden}"
        );
    }
    // The daemon's DELETE exists; this surface never designed a remove
    // control and must not call it by accident. The download literal is
    // built only here, so a DELETE on it could only come from this module.
    assert!(
        !module.contains("Request::delete"),
        "{ATTACHMENTS} has no remove control and must send no DELETE"
    );
    let page = page_src();
    assert_eq!(
        page.matches("/attachments/{}\"").count(),
        1,
        "the attachment download route literal is built in exactly one place"
    );
}

#[test]
fn every_files_class_is_both_styled_and_emitted() {
    let css = read(CSS);
    let rust = page_src();
    let mut styled = BTreeSet::new();
    let mut emitted = BTreeSet::new();
    for prefix in ["rooms-workspace__file", "rooms-workspace__composer-attach"] {
        styled.extend(classes_with_prefix(&css, prefix));
        emitted.extend(classes_with_prefix(&rust, prefix));
    }
    assert!(
        emitted.contains("rooms-workspace__file")
            && emitted.contains("rooms-workspace__files-input")
            && emitted.contains("rooms-workspace__composer-attach"),
        "the scan found none of the Files classes; this scanner broke: {emitted:?}"
    );
    let unstyled: Vec<_> = emitted.difference(&styled).collect();
    assert!(
        unstyled.is_empty(),
        "classes the page emits with no rule in {CSS}: {unstyled:?}"
    );
    let dead: Vec<_> = styled.difference(&emitted).collect();
    assert!(
        dead.is_empty(),
        "selectors in {CSS} no production Rust emits (dead CSS): {dead:?}"
    );
}

#[test]
fn the_files_css_section_takes_its_colors_only_from_tokens() {
    let css = read(CSS);
    let start = css
        .find("Room context files (composer attach + right-rail Files mode)")
        .expect("the Files section header is in rooms-workspace.css");
    let section = &css[start..];
    let bytes = section.as_bytes();
    for (at, _) in section.match_indices('#') {
        let next = bytes.get(at + 1).copied().unwrap_or(b' ');
        assert!(
            !next.is_ascii_hexdigit(),
            "the Files section hard-codes a hex color at offset {at}: {}",
            &section[at..section.len().min(at + 8)]
        );
    }
    for literal in ["rgb(", "rgba(", "hsl(", "hsla("] {
        assert!(
            !section.contains(literal),
            "the Files section hard-codes a {literal} color; colors come only from tokens.css"
        );
    }
    assert!(
        section.contains("var(--err)") && section.contains("var(--fg-3)"),
        "the Files section must colour through tokens"
    );
}

#[test]
fn the_attachments_state_and_file_input_live_at_workspace_scope() {
    let workspace = page_module(WORKSPACE);
    assert!(
        workspace.contains("let attachments = RoomAttachmentsState::new(rooms);"),
        "{WORKSPACE} must construct the attachments state at component scope"
    );
    let input_at = workspace
        .find("<AttachmentInput rooms=rooms state=attachments />")
        .expect("the hidden file input is mounted by RoomsWorkspace");
    let closure_at = workspace
        .find("let open = rooms.open_room.get();")
        .expect("the center-rail open_room closure");
    assert!(
        input_at < closure_at,
        "the file input must be mounted OUTSIDE the open_room closure: a roster write \
         rebuilds that closure and would unmount the input a picker is about to report to"
    );
    // The admission Effect retires the list, its ticket and any upload in
    // flight with the room, exactly where the right-rail modes are cleared.
    let admission = workspace
        .find("members_open.set(false);\n            search_open.set(false);\n            files_open.set(false);")
        .expect("the admission Effect clears the right-rail modes");
    let after = &workspace[admission..admission + 600];
    assert!(
        after.contains("attachments.reset();"),
        "the admission Effect must reset the attachments state with the room"
    );
}

#[test]
fn both_attach_controls_take_the_local_store_gate_not_the_composers() {
    let workspace = page_module(WORKSPACE);
    let local = ["local_store", "_write_gate("].concat();
    let peer = ["access_allows", "_writes("].concat();

    let composer = attribute_window(&workspace, "<ChannelComposer");
    assert!(
        composer.contains("attach_allowed="),
        "the composer must receive the attach gate"
    );
    assert!(
        composer.contains(&local) && !composer.contains(&peer),
        "the composer's attach gate must be {local} — an upload lands in this daemon's store"
    );

    let panel = attribute_window(&workspace, "<FilesPanel");
    assert!(
        panel.contains("writes_allowed=") && panel.contains(&local) && !panel.contains(&peer),
        "the Files panel must take {local}, never {peer}"
    );

    // The composer itself still gates the MESSAGE input on the peer gate: the
    // two rulings coexist in one row on purpose.
    let composer_module = page_module(COMPOSER);
    assert!(
        composer_module
            .contains("disabled=move || !access_allows_writes(rooms.access.get().as_ref())"),
        "the message input keeps the composer's own gate"
    );
    assert!(
        composer_module.contains("upload_control_enabled("),
        "the paperclip must gate through upload_control_enabled"
    );
}

#[test]
fn the_overflow_offers_files_and_the_right_rail_has_a_files_mode() {
    let overflow = page_module(OVERFLOW);
    assert!(
        overflow.contains("on_files.run(());") && overflow.contains("\"Files\""),
        "{OVERFLOW} must offer a Files item that opens the right rail's Files mode"
    );
    let workspace = page_module(WORKSPACE);
    assert!(
        workspace.contains("} else if files_open.get() {\n                            \"Files\"")
            || workspace.contains("files_open.get() {\n                            \"Files\""),
        "the right rail must title its Files mode"
    );
    assert!(
        workspace.contains("\"Close files\""),
        "the right rail's close control must name the Files mode"
    );
    assert!(
        workspace.contains("|| files_open.get()\n"),
        "right_open must include the Files mode so Escape and the close control reach it"
    );
}

#[test]
fn the_client_guards_mirror_the_daemons_constants() {
    let module = page_module(ATTACHMENTS);
    assert!(module.contains("MAX_ATTACHMENT_BYTES: u64 = 8 * 1024 * 1024;"));
    assert!(module.contains("MAX_CONTENT_TYPE_LEN: usize = 128;"));
    assert!(module.contains("MAX_FILENAME_LEN: usize = 128;"));

    let proxy = read(PROXY_MAIN);
    assert!(
        proxy.contains("ATTACHMENT_UPLOAD_BODY_LIMIT: usize = 8 * 1024 * 1024 + 4096;"),
        "the proxy's upload ceiling must be the daemon's cap plus its slack"
    );

    let Ok(daemon) = std::fs::read_to_string(repo_root().join(DAEMON_ATTACHMENTS)) else {
        eprintln!("{DAEMON_ATTACHMENTS} is not beside this checkout; daemon-side parity skipped");
        return;
    };
    for needle in [
        "MAX_ATTACHMENT_BYTES: usize = 8 * 1024 * 1024;",
        "BODY_LIMIT_SLACK: usize = 4096;",
        "MAX_CONTENT_TYPE_LEN: usize = 128;",
        "MAX_FILENAME_LEN: usize = 128;",
        // The query-string metadata contract the page relies on.
        "struct UploadAttachmentQuery",
        "filename: String,",
        "content_type: String,",
        "uploader_id: String,",
        "\"forged_attachment_author\"",
        "\"attachment_too_large\"",
    ] {
        assert!(
            daemon.contains(needle),
            "the daemon's room_attachments.rs no longer carries `{needle}`; \
             re-derive the page's guards in {ATTACHMENTS} against the new contract"
        );
    }
}
