//! Room context files — the browser half of the daemon's attachment routes.
//!
//! A room's attachments are the doc, the spec, the screenshot everybody in the
//! room needs to be looking at. The daemon has owned them since its
//! `room_attachments` module landed; this module is what lets a browser reach
//! them. Ported from Risingtides-dev/ocean-surface `attachments.rs` onto the
//! monorepo's split Rooms layout: the state and pure helpers live here, the
//! attach control sits in the composer (`composer.rs`), the list is the
//! right rail's Files mode (`mod.rs`), and the hidden file input is mounted
//! once at `RoomsWorkspace` scope by [`AttachmentInput`].
//!
//!   GET    /v1/rooms/persistent/{key}/attachments        → the list
//!   POST   /v1/rooms/persistent/{key}/attachments        → put one in
//!   GET    /v1/rooms/persistent/{key}/attachments/{id}   → the bytes
//!
//! The daemon also serves `DELETE .../attachments/{id}`; this surface does not
//! call it (the legacy surface never designed a remove control).
//!
//! Three properties of the wire contract shape everything below:
//!
//! 1. **The upload body is RAW BYTES and every scrap of metadata rides the
//!    query string** — not multipart, not JSON, and emphatically not a custom
//!    header. The daemon's `cors.rs` allows exactly `content-type` and
//!    `authorization` on a cross-origin request, so an `X-Ocean-Attachment-*`
//!    header would pass under curl and die at the browser preflight. The
//!    picked `File` is handed to `fetch` as the body; the browser streams it.
//! 2. **A download is served under a content type DERIVED from the bytes
//!    (PNG/JPEG/GIF/WebP, else `application/octet-stream`) with `nosniff` and
//!    `Content-Disposition: attachment`**, whatever the uploader declared.
//!    Echoing a declared `text/html` back to this origin is stored XSS, so the
//!    declared type is good for exactly one thing here: choosing a glyph. The
//!    bytes are reached by LINKING at that route, never by rendering them.
//! 3. **`uploader_id` is caller-asserted and gated.** An id that resolves to an
//!    Agent or System participant comes back 403 `forged_attachment_author`,
//!    and an id that is not on the roster is refused by the store (403, the
//!    daemon's own words): an agent's file is written by the daemon's convene
//!    path, never by a client claiming its identity. So the control is gated
//!    on a resolved identity and on an access projection that permits a write
//!    to this daemon's own store — [`super::access::local_store_write_gate`],
//!    the composer's SHAPE but deliberately not the composer's gate, because
//!    an upload is never enqueued for a peer.
//!
//! Everything decided before a request goes out — the size ceiling, the empty
//! body, an unusable filename, how a typed refusal should read — is a free
//! function below, unit-testable natively without a browser or a daemon. The
//! daemon stays the authority on every one of those rules; these copies exist
//! so the operator learns the rule instantly instead of watching 8 MiB crawl
//! upstream to a 413.

use gloo_net::http::Request;
use leptos::prelude::*;
use serde::Deserialize;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::spawn_local;

use crate::rooms::{encode, Rooms};

/// Hard ceiling on one attachment, mirroring the daemon's
/// `MAX_ATTACHMENT_BYTES`. Enforced there; repeated here only so the refusal
/// arrives before the upload does.
pub(crate) const MAX_ATTACHMENT_BYTES: u64 = 8 * 1024 * 1024;

/// Longest declared content type the daemon will record
/// (`MAX_CONTENT_TYPE_LEN`).
const MAX_CONTENT_TYPE_LEN: usize = 128;

/// Longest filename the daemon will record (`MAX_FILENAME_LEN`).
const MAX_FILENAME_LEN: usize = 128;

/// What a file with no declared type is called on the wire.
const OPAQUE_CONTENT_TYPE: &str = "application/octet-stream";

// ---- Wire types -------------------------------------------------------------

/// One row of `GET .../attachments`. Mirrors `ocean_core::RoomAttachment`.
///
/// `sha256` and `on_behalf_of` are carried rather than dropped because this
/// struct is the wire shape, not the view model: silently discarding fields
/// the daemon publishes is how a client starts disagreeing with the server
/// about what a record is.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub(crate) struct RoomAttachment {
    pub id: String,
    pub filename: String,
    pub content_type: String,
    pub byte_len: u64,
    #[serde(default)]
    pub sha256: String,
    #[serde(default)]
    pub uploaded_by: String,
    #[serde(default)]
    pub uploaded_at: String,
    #[serde(default)]
    pub on_behalf_of: Option<String>,
}

/// `GET .../attachments`. The monorepo daemon adds `closed` (a soft-closed
/// room is served as an audit read); it is decoded so the shape stays
/// honest, and unused because this surface never holds a closed room open.
#[derive(Debug, Deserialize)]
struct AttachmentsListBody {
    #[serde(default)]
    ok: bool,
    #[serde(default)]
    attachments: Vec<RoomAttachment>,
    #[serde(default)]
    #[allow(dead_code)]
    closed: bool,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    code: Option<String>,
}

/// `POST .../attachments`. A success is `201 { ok, attachment }`; the row is
/// not read off it because the list is re-read instead (see `upload`).
#[derive(Debug, Deserialize)]
struct UploadResultBody {
    #[serde(default)]
    ok: bool,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    code: Option<String>,
}

// ---- Pure helpers -----------------------------------------------------------

fn list_url(base: &str, key: &str) -> String {
    format!("{base}/v1/rooms/persistent/{}/attachments", encode(key))
}

/// The upload URL, with filename / content type / uploader in the QUERY
/// STRING. See the module header for why they cannot be headers.
fn upload_url(base: &str, key: &str, filename: &str, content_type: &str, uploader: &str) -> String {
    format!(
        "{base}/v1/rooms/persistent/{}/attachments?filename={}&content_type={}&uploader_id={}",
        encode(key),
        encode(filename),
        encode(content_type),
        encode(uploader),
    )
}

/// The bytes. Linked, never fetched-and-rendered: the daemon sniffs them and
/// serves a real image type for exactly PNG/JPEG/GIF/WebP, while everything
/// outside that allowlist is `application/octet-stream` with an attachment
/// disposition, landing in the operator's downloads instead of executing on
/// this origin. `nosniff` is unconditional either way.
pub(super) fn download_url(base: &str, key: &str, id: &str) -> String {
    format!(
        "{base}/v1/rooms/persistent/{}/attachments/{}",
        encode(key),
        encode(id),
    )
}

/// The daemon's `sanitize_filename`, mirrored: last path component, no control
/// characters, no `"` (it would break out of the quoted `Content-Disposition`
/// parameter), bounded, and never `.` or `..`.
///
/// The point of running it here is not to sanitize — the daemon does that
/// regardless — but to know BEFORE dispatching whether the name it is about to
/// receive is one it can accept, and to show the operator the name the room
/// will actually record.
fn sanitized_filename(raw: &str) -> Option<String> {
    let last = raw.rsplit(['/', '\\']).next().unwrap_or(raw);
    let cleaned: String = last
        .chars()
        .filter(|c| !c.is_control() && *c != '"')
        .take(MAX_FILENAME_LEN)
        .collect();
    let cleaned = cleaned.trim().to_string();
    if cleaned.is_empty() || cleaned == "." || cleaned == ".." {
        return None;
    }
    Some(cleaned)
}

/// The `content_type` this upload will declare.
///
/// `File.type` is empty whenever the browser cannot guess from the extension,
/// and the daemon refuses an empty declaration outright
/// (`is_storable_content_type`) — so an unlabelled file would 400 with nothing
/// the operator could act on. Fall back to the honest answer, opaque bytes,
/// and apply the daemon's bounded visible-ASCII rule so a browser handing us
/// something stranger is corrected here rather than refused there.
fn declared_content_type(raw: &str) -> String {
    let trimmed = raw.trim();
    let storable = !trimmed.is_empty()
        && trimmed.len() <= MAX_CONTENT_TYPE_LEN
        && trimmed.bytes().all(|b| (0x20..0x7f).contains(&b));
    if storable {
        trimmed.to_string()
    } else {
        OPAQUE_CONTENT_TYPE.to_string()
    }
}

/// Why this file is not going to be uploaded, decided before the request.
///
/// All three rules are the daemon's; refusing here only makes the answer
/// immediate. Nothing is ever ADMITTED on this side — a `None` still faces
/// every server-side check.
fn upload_refusal(raw_filename: &str, byte_len: u64) -> Option<String> {
    let Some(name) = sanitized_filename(raw_filename) else {
        return Some("That file has no usable name.".to_string());
    };
    if byte_len == 0 {
        return Some(format!("\u{201c}{name}\u{201d} is empty."));
    }
    if byte_len > MAX_ATTACHMENT_BYTES {
        return Some(format!(
            "\u{201c}{name}\u{201d} is {} \u{2014} the limit is {}.",
            human_bytes(byte_len),
            human_bytes(MAX_ATTACHMENT_BYTES),
        ));
    }
    None
}

/// A size a person can read at a glance. One decimal past KB because the
/// difference between 1.2 MB and 1.9 MB is the whole reason the label exists.
fn human_bytes(n: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * KB;
    if n >= MB {
        format!("{:.1} MB", n as f64 / MB as f64)
    } else if n >= KB {
        format!("{:.1} KB", n as f64 / KB as f64)
    } else {
        format!("{n} B")
    }
}

/// A glyph for the DECLARED content type.
///
/// This is the only use the declaration gets on this side, and it is
/// deliberately cosmetic: a lying `image/png` buys an attacker a different
/// emoji and nothing else, because the bytes are still fetched from the
/// daemon's sniffed route and never rendered here.
fn kind_glyph(content_type: &str) -> &'static str {
    let lowered = content_type.to_ascii_lowercase();
    if lowered.starts_with("image/") {
        "\u{1f5bc}"
    } else if lowered.starts_with("video/") {
        "\u{1f3ac}"
    } else if lowered.starts_with("audio/") {
        "\u{1f50a}"
    } else if lowered.contains("pdf") {
        "\u{1f4d5}"
    } else if lowered.starts_with("text/") || lowered.contains("json") || lowered.contains("xml") {
        "\u{1f4c4}"
    } else if lowered.contains("zip") || lowered.contains("tar") || lowered.contains("gzip") {
        "\u{1f5dc}"
    } else {
        "\u{1f4ce}"
    }
}

/// Turn a refusal into something an operator can act on.
///
/// The daemon's typed `code` is the input, not its prose: the codes are stable
/// and the prose is written for a log reader. `forged_attachment_author` in
/// particular has to say what actually happened — the control was live because
/// this identity may write, but the id it carried belongs to an agent — or the
/// operator reads a permission bug into a working gate. An untyped 413 is the
/// proxy's own body ceiling (plain text, no JSON) and reads as the same rule.
/// Store refusals (`ok:false` with the daemon's `error` and no `code`, e.g. an
/// uploader who is not on the roster) keep the daemon's words.
fn failure_message(prefix: &str, status: u16, code: Option<&str>, error: Option<&str>) -> String {
    match code {
        Some("attachment_too_large") => format!(
            "The room refused that file: the limit is {}.",
            human_bytes(MAX_ATTACHMENT_BYTES)
        ),
        Some("forged_attachment_author") => {
            "An agent's file is written by the daemon, not uploaded on its behalf.".to_string()
        }
        Some("unknown_room") => "That room is no longer open.".to_string(),
        Some("invalid_request") => "The room refused that file's name or type.".to_string(),
        _ => match (status, error) {
            (413, _) => format!(
                "The room refused that file: the limit is {}.",
                human_bytes(MAX_ATTACHMENT_BYTES)
            ),
            (_, Some(text)) if !text.is_empty() => format!("{prefix}: {text}"),
            _ => format!("{prefix} ({status})."),
        },
    }
}

const LIST_FAILED: &str = "Could not load files";
const UPLOAD_FAILED: &str = "Upload failed";

/// Fold a list response into rows or a sentence.
///
/// Status first: a non-2xx that does not decode (the proxy's plain-text
/// refusals, an HTML error page from a tunnel) still gets a status-worded
/// answer instead of a serde message about a `{` that was not there.
fn fold_list_response(status: u16, body: &str) -> Result<Vec<RoomAttachment>, String> {
    match serde_json::from_str::<AttachmentsListBody>(body) {
        Ok(parsed) if parsed.ok => Ok(parsed.attachments),
        Ok(parsed) => Err(failure_message(
            LIST_FAILED,
            status,
            parsed.code.as_deref(),
            parsed.error.as_deref(),
        )),
        Err(_) if status >= 400 => Err(failure_message(LIST_FAILED, status, None, None)),
        Err(err) => Err(format!("Files decode error: {err}")),
    }
}

/// Fold an upload response into success or a sentence. Same status-first rule
/// as [`fold_list_response`]; the 413 arm matters most here, because the
/// proxy's body ceiling answers in plain text.
fn fold_upload_response(status: u16, body: &str) -> Result<(), String> {
    match serde_json::from_str::<UploadResultBody>(body) {
        Ok(parsed) if parsed.ok => Ok(()),
        Ok(parsed) => Err(failure_message(
            UPLOAD_FAILED,
            status,
            parsed.code.as_deref(),
            parsed.error.as_deref(),
        )),
        Err(_) if status >= 400 => Err(failure_message(UPLOAD_FAILED, status, None, None)),
        Err(err) => Err(format!("Upload decode error: {err}")),
    }
}

/// Latest-wins admission for an overlapping list fetch. Identical in shape to
/// `rooms.rs`'s list ticket, and here for the same reason: an older completion
/// publishing over a newer one is what put a premature "no files" on screen in
/// three previous legacy features.
fn list_request_is_current(ticket: u64, current: u64) -> bool {
    ticket == current
}

/// Whether the attach controls (composer paperclip, panel `+ file`) are live.
///
/// Identity is deliberately NOT part of this predicate: the owner identity
/// loads asynchronously and a control disabled on it would never re-enable
/// when bootstrap answers, so the file picker gates on access at the control
/// and refuses on identity at the action — exactly like the composer, with
/// its own access gate rather than the composer's.
pub(super) fn upload_control_enabled(
    writes_allowed: bool,
    uploading: bool,
    open_key: Option<&str>,
) -> bool {
    writes_allowed && !uploading && open_key.is_some_and(|key| !key.is_empty())
}

/// The one fact the panel head states: that files exist, and how many.
fn file_count_line(count: usize) -> String {
    if count == 1 {
        "1 file".to_string()
    } else {
        format!("{count} files")
    }
}

// ---- State ------------------------------------------------------------------

/// Reactive handle for one room's context files.
///
/// Constructed at `RoomsWorkspace` component scope, never inside a rail
/// closure: those closures re-run on every `rooms.access` SSE update and on
/// every `open_room` write, and an upload's in-flight flag rebuilt
/// mid-request would leave the control enabled during its own upload.
#[derive(Clone, Copy)]
pub(crate) struct RoomAttachmentsState {
    /// Daemon base URL, shared with `Daemon::url` through `Rooms::url` — read
    /// live at request time because bootstrap resolves the origin
    /// asynchronously (a phone via the tunnel resolves it late).
    pub url: RwSignal<String>,
    /// The open room's attachments, newest-first as the daemon ordered them.
    pub items: RwSignal<Vec<RoomAttachment>>,
    /// Whether a list request has SUCCEEDED for the room now open. Starts
    /// false and returns to false on every room change, so the empty state can
    /// never assert "no files" about a room that has not answered yet.
    pub loaded: RwSignal<bool>,
    /// Whether a list request is in flight.
    pub loading: RwSignal<bool>,
    /// The most recent failure, list or upload. Rendered under the composer
    /// AND in the panel: a failure while the panel is closed must not vanish.
    pub error: RwSignal<Option<String>>,
    /// An upload is in flight — blocks re-submit and drives the button label.
    pub uploading: RwSignal<bool>,
    /// The real file control ([`AttachmentInput`]), mounted once at workspace
    /// scope so a gesture forwarded from the composer or the panel always has
    /// somewhere to land, whatever the center rail is doing.
    pub(super) input_ref: NodeRef<leptos::html::Input>,
    /// Monotonic ticket; only the latest overlapping list request may publish.
    ticket: RwSignal<u64>,
}

impl RoomAttachmentsState {
    pub(super) fn new(rooms: Rooms) -> Self {
        Self {
            url: rooms.url,
            items: RwSignal::new(Vec::new()),
            loaded: RwSignal::new(false),
            loading: RwSignal::new(false),
            error: RwSignal::new(None),
            uploading: RwSignal::new(false),
            input_ref: NodeRef::new(),
            ticket: RwSignal::new(0),
        }
    }

    fn base(&self) -> String {
        self.url.get_untracked().trim_end_matches('/').to_string()
    }

    /// Open the native file picker: forward the user gesture to the mounted
    /// input. One file per upload — the route takes one raw body.
    pub(super) fn open_picker(&self) {
        if let Some(input) = self.input_ref.get_untracked() {
            input.click();
        }
    }

    /// Retire whatever is on screen and whatever is in flight.
    ///
    /// The ticket bump is the load-bearing half: without it the previous
    /// room's list could still land and be rendered under this room's name.
    /// `uploading` is retired here as well: an upload belongs to the room it
    /// started in, so a flag carried across a room change disables the new
    /// room's control over a file nobody ever sent it — permanently, if the
    /// old request never resolves.
    pub(super) fn reset(&self) {
        self.ticket
            .update(|ticket| *ticket = ticket.wrapping_add(1));
        self.items.set(Vec::new());
        self.loaded.set(false);
        self.loading.set(false);
        self.error.set(None);
        self.uploading.set(false);
    }

    /// Load one room's file list.
    pub(super) fn fetch(&self, key: String) {
        let base = self.base();
        let me = *self;
        let ticket = self.ticket.get_untracked().wrapping_add(1);
        self.ticket.set(ticket);
        self.loading.set(true);
        self.error.set(None);
        spawn_local(async move {
            let url = list_url(&base, &key);
            let result = match Request::get(&url).send().await {
                Ok(resp) => {
                    let status = resp.status();
                    let text = resp.text().await.unwrap_or_default();
                    fold_list_response(status, &text)
                }
                Err(err) => Err(format!("Files request failed: {err}")),
            };
            if !list_request_is_current(ticket, me.ticket.get_untracked()) {
                return;
            }
            me.loading.set(false);
            match result {
                Ok(items) => {
                    me.items.set(items);
                    // Only a SUCCESS may declare the list known. A failed fetch
                    // that flipped this would replace an honest error with the
                    // false claim that the room has no files.
                    me.loaded.set(true);
                }
                Err(error) => me.error.set(Some(error)),
            }
        });
    }

    /// POST a picked file's bytes, then re-list.
    ///
    /// The refusal check runs on `File.size` before anything is read, so an
    /// oversize file is never even handed to `fetch`, let alone sent. The
    /// `File` itself is the request body: the browser streams it and the
    /// daemon ignores the body's content type (metadata is in the query).
    ///
    /// `rooms` is taken so the completion can re-validate the `(generation,
    /// key)` pair it started with. The list ticket cannot stand in for that
    /// here: the success arm calls `fetch`, which mints a FRESH ticket, so a
    /// room change mid-upload would republish the old room's files — with
    /// hrefs built from the new room's key — under the new room's name.
    fn upload(&self, rooms: Rooms, key: String, uploader: String, file: web_sys::File) {
        let raw_name = file.name();
        let byte_len = file.size().max(0.0) as u64;
        if let Some(refusal) = upload_refusal(&raw_name, byte_len) {
            self.error.set(Some(refusal));
            return;
        }
        let Some(filename) = sanitized_filename(&raw_name) else {
            return;
        };
        let content_type = declared_content_type(&file.type_());
        let base = self.base();
        let me = *self;
        let generation = rooms.generation_snapshot();
        self.uploading.set(true);
        self.error.set(None);
        spawn_local(async move {
            let url = upload_url(&base, &key, &filename, &content_type, &uploader);
            let outcome = match Request::post(&url).body(file) {
                Ok(request) => match request.send().await {
                    Ok(resp) => {
                        let status = resp.status();
                        let text = resp.text().await.unwrap_or_default();
                        fold_upload_response(status, &text)
                    }
                    Err(err) => Err(format!("Upload request failed: {err}")),
                },
                Err(err) => Err(format!("Upload encode error: {err}")),
            };
            // Every write below belongs to the room this upload started in.
            // If that room is gone, `reset` has already cleared this state for
            // whoever is on screen now, and the bytes still landed in the room
            // they were meant for — reopening it shows the file.
            if !rooms.room_is_current(generation, &key) {
                return;
            }
            me.uploading.set(false);
            match outcome {
                // The daemon appends a transcript marker for the upload (a
                // System row the tail delivers), but the row does not carry
                // the attachment itself, so the list is re-read rather than
                // patched locally.
                Ok(()) => me.fetch(key),
                Err(error) => me.error.set(Some(error)),
            }
        });
    }

    /// The `change` handler of the mounted input: pick, validate, upload.
    fn on_pick(&self, rooms: Rooms, ev: &web_sys::Event) {
        let Some(target) = ev.target() else { return };
        let Ok(input) = target.dyn_into::<web_sys::HtmlInputElement>() else {
            return;
        };
        let picked = input.files().and_then(|files| files.get(0));
        // Let the operator pick the same file again after a failure; browsers
        // suppress `change` for an unchanged value.
        input.set_value("");
        let Some(file) = picked else { return };
        let Some(key) = rooms.open_key.get_untracked().filter(|key| !key.is_empty()) else {
            return;
        };
        // Identity is daemon-owned (`GET /v1/me`); the roster-asserted
        // `uploader_id` is that owner. Access gated the control; identity is
        // refused at the action so a late `/v1/me` cannot wedge the button.
        let uploader = rooms.identity_id.get_untracked().trim().to_string();
        if uploader.is_empty() {
            self.error.set(Some(
                "Still signing in \u{2014} try again in a moment.".to_string(),
            ));
            return;
        }
        self.upload(rooms, key, uploader, file);
    }
}

// ---- Components -------------------------------------------------------------

/// The real file control. Mounted once at `RoomsWorkspace` scope, outside the
/// `open_room` closure that rebuilds the center rail on every roster write,
/// so a picker opened from the composer still has an input to report to when
/// the dialog closes.
#[component]
pub(super) fn AttachmentInput(rooms: Rooms, state: RoomAttachmentsState) -> impl IntoView {
    view! {
        <input
            class="rooms-workspace__files-input"
            type="file"
            aria-label="Choose a room context file"
            tabindex="-1"
            node_ref=state.input_ref
            on:change=move |ev| state.on_pick(rooms, &ev)
        />
    }
}

/// The right rail's Files mode: the open room's context files at a reading
/// measure, with the panel's own add control.
///
/// `writes_allowed` is supplied by the workspace rather than recomputed here
/// so one place holds the ruling. It is deliberately NOT the composer's gate:
/// an upload writes bytes and a row on this host and is never enqueued for a
/// peer, so a link that is down or coming back does not hold it
/// (`local_store_write_gate`).
#[component]
pub(super) fn FilesPanel(
    rooms: Rooms,
    state: RoomAttachmentsState,
    writes_allowed: Signal<bool>,
) -> impl IntoView {
    // The list loads when the panel is asked for, like a work card's steps;
    // a room switch closes the panel through the workspace's admission reset.
    Effect::new(move |_| {
        if let Some(key) = rooms.open_key.get().filter(|key| !key.is_empty()) {
            state.fetch(key);
        }
    });

    let can_upload = move || {
        upload_control_enabled(
            writes_allowed.get(),
            state.uploading.get(),
            rooms.open_key.get().as_deref(),
        )
    };

    view! {
        <div class="rooms-workspace__files">
            <div class="rooms-workspace__files-head">
                // Order matters: in-flight and never-answered both outrank
                // the empty state, which may only speak for a room that
                // replied.
                <span class="rooms-workspace__files-line">
                    {move || {
                        if state.loading.get() {
                            "Loading files\u{2026}".to_string()
                        } else if !state.loaded.get() {
                            String::new()
                        } else {
                            let count = state.items.with(|items| items.len());
                            if count == 0 {
                                "No files yet.".to_string()
                            } else {
                                file_count_line(count)
                            }
                        }
                    }}
                </span>
                // Forwards the user gesture to the mounted input; see
                // `AttachmentInput`.
                <button
                    class="rooms-workspace__files-add"
                    type="button"
                    title="Attach a context file to this room"
                    disabled=move || !can_upload()
                    on:click=move |_| state.open_picker()
                >
                    {move || {
                        if state.uploading.get() {
                            "uploading\u{2026}"
                        } else {
                            "+ file"
                        }
                    }}
                </button>
            </div>
            {move || {
                state.error.get().map(|error| view! {
                    <div class="rooms-workspace__files-error" role="alert">{error}</div>
                })
            }}
            {move || {
                if !state.loaded.get() {
                    return ().into_any();
                }
                let items = state.items.get();
                if items.is_empty() {
                    return ().into_any();
                }
                let base = state.url.get().trim_end_matches('/').to_string();
                let key = rooms.open_key.get().unwrap_or_default();
                let rows = items
                    .into_iter()
                    .map(|item| {
                        // The href is the daemon's download route. `download`
                        // is belt to its braces: same-origin the browser
                        // honors it, cross-origin (Tauri, extension) the
                        // daemon's own Content-Disposition still lands the
                        // file.
                        let href = download_url(&base, &key, &item.id);
                        let meta = format!(
                            "{} \u{b7} {}",
                            human_bytes(item.byte_len),
                            item.uploaded_by,
                        );
                        view! {
                            <a
                                class="rooms-workspace__file"
                                role="listitem"
                                href=href
                                download=item.filename.clone()
                                title=format!("{} ({})", item.filename, item.content_type)
                            >
                                <span class="rooms-workspace__file-glyph" aria-hidden="true">
                                    {kind_glyph(&item.content_type)}
                                </span>
                                <span class="rooms-workspace__file-name">
                                    {item.filename.clone()}
                                </span>
                                <span class="rooms-workspace__file-meta">{meta}</span>
                            </a>
                        }
                    })
                    .collect::<Vec<_>>();
                view! {
                    <div class="rooms-workspace__files-list" role="list" aria-label="Room files">
                        {rows}
                    </div>
                }
                .into_any()
            }}
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rooms::room_request_is_current;

    #[test]
    fn upload_url_carries_every_field_in_the_query_string() {
        // Not headers: cors.rs allows only content-type and authorization, so a
        // custom header passes under curl and dies at the browser preflight.
        let url = upload_url(
            "http://d",
            "call:xyz",
            "spec v2.md",
            "text/markdown",
            "smaths",
        );
        assert_eq!(
            url,
            "http://d/v1/rooms/persistent/call%3Axyz/attachments\
             ?filename=spec%20v2.md&content_type=text%2Fmarkdown&uploader_id=smaths"
        );
    }

    #[test]
    fn room_key_is_one_encoded_path_segment_on_every_route() {
        // A key with a slash must never become two segments and reach a
        // different daemon route.
        assert_eq!(
            list_url("http://d", "a/b"),
            "http://d/v1/rooms/persistent/a%2Fb/attachments"
        );
        assert_eq!(
            download_url("http://d", "a/b", "0123456789abcdef0123456789abcdef"),
            "http://d/v1/rooms/persistent/a%2Fb/attachments/0123456789abcdef0123456789abcdef"
        );
    }

    #[test]
    fn oversize_and_empty_files_are_refused_before_the_request() {
        let refusal = upload_refusal("huge.bin", MAX_ATTACHMENT_BYTES + 1).expect("refused");
        assert!(refusal.contains("8.0 MB"), "names the limit: {refusal}");
        assert!(refusal.contains("huge.bin"), "names the file: {refusal}");
        assert!(upload_refusal("empty.txt", 0).is_some());
        // Exactly at the cap is legal — the daemon admits it, so we must too.
        assert!(upload_refusal("exact.bin", MAX_ATTACHMENT_BYTES).is_none());
        assert!(upload_refusal("notes.md", 12).is_none());
    }

    #[test]
    fn a_name_the_daemon_would_reject_is_refused_here_instead() {
        assert!(upload_refusal("..", 10).is_some());
        assert!(upload_refusal(".", 10).is_some());
        assert!(upload_refusal("   ", 10).is_some());
        assert!(upload_refusal("/", 10).is_some());
    }

    #[test]
    fn filenames_are_sanitized_the_way_the_daemon_sanitizes_them() {
        assert_eq!(sanitized_filename("/etc/passwd").as_deref(), Some("passwd"));
        assert_eq!(
            sanitized_filename("C:\\docs\\spec.md").as_deref(),
            Some("spec.md")
        );
        // A quote would break out of the Content-Disposition parameter.
        assert_eq!(sanitized_filename("a\"b.txt").as_deref(), Some("ab.txt"));
        assert_eq!(sanitized_filename("a\nb.txt").as_deref(), Some("ab.txt"));
        assert_eq!(
            sanitized_filename(&"x".repeat(200)).map(|name| name.len()),
            Some(MAX_FILENAME_LEN)
        );
        assert_eq!(sanitized_filename(""), None);
    }

    #[test]
    fn an_undeclared_type_becomes_opaque_bytes_not_an_empty_string() {
        // The daemon refuses an empty content_type outright, so sending one
        // through would be a 400 the operator could do nothing about.
        assert_eq!(declared_content_type(""), OPAQUE_CONTENT_TYPE);
        assert_eq!(declared_content_type("   "), OPAQUE_CONTENT_TYPE);
        assert_eq!(declared_content_type("text/markdown"), "text/markdown");
        // Bounded and visible-ASCII, matching is_storable_content_type.
        assert_eq!(declared_content_type("text/\u{e9}"), OPAQUE_CONTENT_TYPE);
        assert_eq!(declared_content_type("text/x\nhtml"), OPAQUE_CONTENT_TYPE);
        assert_eq!(
            declared_content_type(&"a".repeat(MAX_CONTENT_TYPE_LEN + 1)),
            OPAQUE_CONTENT_TYPE
        );
    }

    #[test]
    fn typed_refusals_read_as_the_rule_they_are() {
        assert!(
            failure_message(UPLOAD_FAILED, 413, Some("attachment_too_large"), None)
                .contains("8.0 MB")
        );
        let forged = failure_message(UPLOAD_FAILED, 403, Some("forged_attachment_author"), None);
        assert!(forged.contains("daemon"), "{forged}");
        // The proxy's own body ceiling answers an untyped plain-text 413; it
        // is the same rule and must read as one.
        assert!(failure_message(UPLOAD_FAILED, 413, None, None).contains("8.0 MB"));
        // An untyped failure still says something, and never swallows the
        // server's own words when it has them.
        assert_eq!(
            failure_message(UPLOAD_FAILED, 500, None, Some("disk on fire")),
            "Upload failed: disk on fire"
        );
        assert_eq!(
            failure_message(UPLOAD_FAILED, 500, None, None),
            "Upload failed (500)."
        );
        assert_eq!(
            failure_message(UPLOAD_FAILED, 500, None, Some("")),
            "Upload failed (500)."
        );
    }

    #[test]
    fn a_list_response_folds_from_the_daemons_own_shape() {
        let rows = fold_list_response(
            200,
            r#"{"ok":true,"closed":false,"attachments":[{
                "id":"0123456789abcdef0123456789abcdef",
                "filename":"spec.md",
                "content_type":"text/markdown",
                "byte_len":1234,
                "sha256":"deadbeef",
                "uploaded_by":"smaths",
                "uploaded_at":"2026-08-25T10:00:00Z"
            }]}"#,
        )
        .expect("decode");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].filename, "spec.md");
        // on_behalf_of is absent over HTTP today (the forged-author gate means
        // only a human ever uploads) and must not be required to decode.
        assert_eq!(rows[0].on_behalf_of, None);

        // The store's unknown-room refusal has no `code`, only the daemon's
        // words, and they survive.
        assert_eq!(
            fold_list_response(404, r#"{"ok":false,"error":"unknown room: gone"}"#),
            Err("Could not load files: unknown room: gone".to_string())
        );
        // A refusal that is not JSON at all (a proxy or tunnel page) is worded
        // by status, never as a serde complaint.
        assert_eq!(
            fold_list_response(502, "<html>bad gateway</html>"),
            Err("Could not load files (502).".to_string())
        );
        // A 200 that does not decode IS a decode error — the daemon changed.
        assert!(fold_list_response(200, "nope")
            .unwrap_err()
            .starts_with("Files decode error"));
    }

    #[test]
    fn an_upload_response_folds_from_the_daemons_own_shape() {
        // 201 with the row: success, and the row is not needed here.
        assert_eq!(
            fold_upload_response(
                201,
                r#"{"ok":true,"attachment":{"id":"0123456789abcdef0123456789abcdef","filename":"a","content_type":"text/plain","byte_len":1,"sha256":"","uploaded_by":"me","uploaded_at":""}}"#,
            ),
            Ok(())
        );
        // The typed refusal body decodes without the success fields.
        let too_large = fold_upload_response(
            413,
            r#"{"ok":false,"code":"attachment_too_large","error":"too big","max_bytes":8388608}"#,
        )
        .unwrap_err();
        assert!(too_large.contains("8.0 MB"), "{too_large}");
        // The proxy's body ceiling: plain text, no JSON, same rule.
        let proxy_cap = fold_upload_response(413, "request body too large").unwrap_err();
        assert!(proxy_cap.contains("8.0 MB"), "{proxy_cap}");
        // The store's roster refusal carries no code; the daemon's words show.
        assert_eq!(
            fold_upload_response(
                403,
                r#"{"ok":false,"error":"attachment uploader 'x' is not in room 'team'"}"#
            ),
            Err("Upload failed: attachment uploader 'x' is not in room 'team'".to_string())
        );
        assert_eq!(
            fold_upload_response(
                403,
                r#"{"ok":false,"code":"forged_attachment_author","error":"..."}"#
            ),
            Err(
                "An agent's file is written by the daemon, not uploaded on its behalf.".to_string()
            )
        );
    }

    #[test]
    fn stale_list_responses_cannot_publish() {
        assert!(list_request_is_current(7, 7));
        assert!(!list_request_is_current(6, 7));
    }

    /// The ticket above cannot cover an upload: its success arm calls `fetch`,
    /// which mints a fresh ticket and would therefore be admitted. So the
    /// completion re-validates the `(generation, key)` pair it started with
    /// through `Rooms::room_is_current`, which delegates to
    /// `crate::rooms::room_request_is_current` — exercised directly here, with
    /// no live `Rooms` or browser runtime needed.
    #[test]
    fn an_upload_landing_after_a_room_change_cannot_publish() {
        let started_generation = 3;
        let started_key = "room-a";

        // It lands while A is still the open room: publish.
        assert!(room_request_is_current(
            started_generation,
            started_generation,
            started_key,
            Some(started_key),
        ));
        // The operator switched to room B first. Publishing here would paint
        // A's filenames under B's name, with hrefs built from B's key.
        assert!(!room_request_is_current(
            started_generation,
            started_generation + 1,
            started_key,
            Some("room-b"),
        ));
        // Closed to no room at all: still nothing to publish into.
        assert!(!room_request_is_current(
            started_generation,
            started_generation + 1,
            started_key,
            None,
        ));
        // A close/reopen of the SAME key is a different admission — `reset`
        // has already cleared this state, so the in-flight upload must not
        // write into it even though the key still matches.
        assert!(!room_request_is_current(
            started_generation,
            started_generation + 1,
            started_key,
            Some(started_key),
        ));
    }

    #[test]
    fn reset_retires_the_in_flight_upload_not_just_the_list() {
        let owner = Owner::new();
        owner.set();
        let state = RoomAttachmentsState {
            url: RwSignal::new("http://d".to_string()),
            items: RwSignal::new(vec![RoomAttachment {
                id: "0123456789abcdef0123456789abcdef".to_string(),
                filename: "spec.md".to_string(),
                content_type: "text/markdown".to_string(),
                byte_len: 12,
                sha256: String::new(),
                uploaded_by: "smaths".to_string(),
                uploaded_at: String::new(),
                on_behalf_of: None,
            }]),
            loaded: RwSignal::new(true),
            loading: RwSignal::new(true),
            error: RwSignal::new(Some("boom".to_string())),
            uploading: RwSignal::new(true),
            input_ref: NodeRef::new(),
            ticket: RwSignal::new(4),
        };

        state.reset();

        assert!(state.items.get_untracked().is_empty());
        assert!(!state.loaded.get_untracked());
        assert!(!state.loading.get_untracked());
        assert!(state.error.get_untracked().is_none());
        // The room the upload was for is gone; leaving this true renders the
        // NEXT room's control disabled and reading "uploading…" for a file it
        // never saw, forever if that request never resolves.
        assert!(!state.uploading.get_untracked());
        // And the ticket still moves, so a list in flight cannot land either.
        assert_eq!(state.ticket.get_untracked(), 5);
    }

    #[test]
    fn the_attach_control_gates_on_access_flight_and_an_open_room() {
        assert!(upload_control_enabled(true, false, Some("team")));
        assert!(!upload_control_enabled(false, false, Some("team")));
        assert!(!upload_control_enabled(true, true, Some("team")));
        assert!(!upload_control_enabled(true, false, None));
        assert!(!upload_control_enabled(true, false, Some("")));
    }

    #[test]
    fn the_panel_head_counts_its_files() {
        assert_eq!(file_count_line(1), "1 file");
        assert_eq!(file_count_line(2), "2 files");
        assert_eq!(file_count_line(12), "12 files");
    }

    #[test]
    fn sizes_read_like_sizes() {
        assert_eq!(human_bytes(0), "0 B");
        assert_eq!(human_bytes(512), "512 B");
        assert_eq!(human_bytes(2048), "2.0 KB");
        assert_eq!(human_bytes(MAX_ATTACHMENT_BYTES), "8.0 MB");
    }

    #[test]
    fn the_glyph_is_the_only_thing_a_declared_type_decides() {
        assert_eq!(kind_glyph("image/png"), kind_glyph("IMAGE/PNG"));
        assert_ne!(kind_glyph("image/png"), kind_glyph("application/pdf"));
        // A lie about the type buys a different emoji and nothing more: the
        // bytes are still fetched from the daemon's sniffed route.
        assert_eq!(kind_glyph("text/html"), kind_glyph("text/plain"));
    }
}
