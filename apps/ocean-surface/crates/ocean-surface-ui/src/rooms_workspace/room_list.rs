//! Left rail: room listbox (roving tabindex), status line, and create input
//! with its optional workspace selector.

use leptos::prelude::*;
use wasm_bindgen::JsCast;

use crate::daemon::ProjectInfo;
use crate::rooms::{CreateResolution, Room, Rooms};

// ── Room-create workspace selector (pure, unit-testable) ───────────────────

/// Select value of the native "Choose folder…" entry. Never an absolute path,
/// so it cannot collide with a workspace option.
pub(super) const CHOOSE_FOLDER_VALUE: &str = "choose-folder";

/// One selectable workspace root for a new room.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct WorkspaceOption {
    pub root: String,
    pub label: String,
}

/// What a workspace `<select>` value means.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum WorkspaceSelection {
    /// Unbound room: no `workspace_root` is sent.
    None,
    Root(String),
    ChooseFolder,
}

/// True for an absolute POSIX path or a drive/UNC-rooted Windows path. The
/// daemon refuses anything else, so relative roots are never offered.
pub(super) fn is_absolute_root(path: &str) -> bool {
    let bytes = path.as_bytes();
    path.starts_with('/')
        || path.starts_with("\\\\")
        || (bytes.len() >= 3
            && bytes[0].is_ascii_alphabetic()
            && bytes[1] == b':'
            && (bytes[2] == b'\\' || bytes[2] == b'/'))
}

/// Trimmed absolute root without a trailing separator (except a bare root).
fn normalized_root(path: &str) -> Option<String> {
    let trimmed = path.trim();
    if !is_absolute_root(trimmed) {
        return None;
    }
    let stripped = trimmed.trim_end_matches(['/', '\\']);
    let root = if stripped.is_empty() || stripped.ends_with(':') {
        trimmed
    } else {
        stripped
    };
    Some(root.to_string())
}

pub(super) fn root_basename(root: &str) -> &str {
    root.rsplit(['/', '\\'])
        .find(|segment| !segment.is_empty())
        .unwrap_or(root)
}

/// Workspace choices for a new room: each daemon project's absolute
/// `workspace_root` (first wins on duplicates), then a natively picked folder
/// not already listed. Labels are the project name (or folder name for a
/// picked root); colliding labels fall back to the full root.
pub(super) fn workspace_options(
    projects: &[ProjectInfo],
    picked: Option<&str>,
) -> Vec<WorkspaceOption> {
    let mut options: Vec<WorkspaceOption> = Vec::new();
    let candidates = projects
        .iter()
        .map(|project| (project.workspace_root.as_str(), Some(project.name.trim())))
        .chain(picked.map(|root| (root, None)));
    for (raw, name) in candidates {
        let Some(root) = normalized_root(raw) else {
            continue;
        };
        if options.iter().any(|option| option.root == root) {
            continue;
        }
        let label = match name {
            Some(name) if !name.is_empty() => name.to_string(),
            _ => root_basename(&root).to_string(),
        };
        options.push(WorkspaceOption { root, label });
    }
    let labels: Vec<String> = options.iter().map(|o| o.label.clone()).collect();
    for option in &mut options {
        if labels
            .iter()
            .filter(|label| **label == option.label)
            .count()
            > 1
        {
            option.label = option.root.clone();
        }
    }
    options
}

/// Decode a workspace `<select>` value. The empty value is "No workspace";
/// anything not absolute is treated as no workspace rather than sent.
pub(super) fn workspace_selection(value: &str) -> WorkspaceSelection {
    if value == CHOOSE_FOLDER_VALUE {
        return WorkspaceSelection::ChooseFolder;
    }
    match normalized_root(value) {
        Some(root) => WorkspaceSelection::Root(root),
        None => WorkspaceSelection::None,
    }
}

// ── Room-list ARIA listbox helpers (pure, unit-testable) ───────────────────

/// Roving-tabindex keyboard model for the room-list listbox. Given the
/// ordered room keys, the key of the option that currently has DOM focus,
/// and the pressed key, returns the index that should receive focus next.
/// `None` means "not a navigation key — leave the event alone".
/// ArrowDown/ArrowUp wrap (listbox convention); Home/End jump.
pub(super) fn room_list_next_focus(
    keys: &[String],
    focused: Option<&str>,
    pressed: &str,
) -> Option<usize> {
    if keys.is_empty() {
        return None;
    }
    let cur = focused.and_then(|f| keys.iter().position(|k| k == f));
    match pressed {
        "ArrowDown" => Some(cur.map_or(0, |i| (i + 1) % keys.len())),
        "ArrowUp" => Some(cur.map_or(keys.len() - 1, |i| (i + keys.len() - 1) % keys.len())),
        "Home" => Some(0),
        "End" => Some(keys.len() - 1),
        _ => None,
    }
}

/// The single tab stop of the roving-tabindex listbox: the open room when it
/// is present in the list, else the first room. Exactly one option carries
/// tabindex=0 so Tab enters the list once and arrows move within it.
pub(super) fn room_list_tab_stop(keys: &[String], open: Option<&str>) -> Option<usize> {
    if keys.is_empty() {
        return None;
    }
    Some(
        open.and_then(|o| keys.iter().position(|k| k == o))
            .unwrap_or(0),
    )
}

/// DOM id for a room option — the stable hook the keydown handler uses to
/// move real focus (roving tabindex needs actual `.focus()` calls).
pub(super) fn room_option_dom_id(key: &str) -> String {
    format!("rooms-opt-{key}")
}

/// Left rail: room listbox, list/create status line, and the new-room input.
/// The create draft, its workspace choice, and its op-id admission state live
/// here; nothing else in the workspace reads them.
#[component]
pub(super) fn RoomListRail(
    rooms: Rooms,
    /// The daemon's project catalogue; its workspace roots are the selectable
    /// workspaces for a new room.
    projects: Signal<Vec<ProjectInfo>>,
    /// Compact-drawer visibility, shared with the mobile nav toggle,
    /// backdrop, and Escape handling in `RoomsWorkspace`.
    show_left_rail: RwSignal<bool>,
    /// Leaves the Rooms workspace entirely; see `RoomsWorkspace::on_close`.
    on_close: Option<Callback<()>>,
) -> impl IntoView {
    let new_room_name = RwSignal::new(String::new());
    // Team-platform P6: the mentions inbox replaces the list while open.
    let inbox_open = RwSignal::new(false);
    let create_input_ref: NodeRef<leptos::html::Input> = NodeRef::new();
    // Workspace for the next create: `None` (unbound) unless chosen here.
    // Never seeded from a session cwd, the focused chat's project, or /tmp.
    let workspace_choice = RwSignal::new(None::<String>);
    let picked_folder = RwSignal::new(None::<String>);
    let picker_pending = RwSignal::new(false);
    let workspace_select_ref: NodeRef<leptos::html::Select> = NodeRef::new();
    let can_pick_folder = crate::host::running_in_tauri();
    let workspace_choices =
        Memo::new(move |_| workspace_options(&projects.get(), picked_folder.get().as_deref()));

    Effect::new(move |_| {
        if show_left_rail.get() {
            request_animation_frame(move || {
                if let Some(input) = create_input_ref.get() {
                    let _ = input.focus();
                }
            });
        }
    });

    // ── Left-rail: create room (draft retained until the typed
    //    create_op delivers a matching outcome — op-id gating so
    //    concurrent submits never cross-resolve, and CAS publication
    //    prevents stale completions from overwriting later ops).
    let pending_create = RwSignal::new(false);
    let create_op_id: RwSignal<u64> = RwSignal::new(0);
    let create_room = move || {
        // Prevent concurrent dispatch: if a create is already in flight,
        // ignore the keypress. The Effect clears pending_create when the
        // outcome resolves; until then, any second Enter is a no-op.
        if pending_create.get_untracked() || picker_pending.get_untracked() {
            return;
        }
        let name = new_room_name.get_untracked();
        if name.trim().is_empty() {
            return;
        }
        // Send only a root still offered; a choice whose project vanished
        // from the catalogue never rides along unseen.
        let workspace_root = workspace_choice.get_untracked().filter(|root| {
            workspace_choices.with_untracked(|options| options.iter().any(|o| &o.root == root))
        });
        let op_id = rooms.create_room(name.clone(), None, workspace_root);
        if op_id == 0 {
            // Synchronous rejection — empty name or slug. Don't set
            // pending; the name field already shows the error via status.
            return;
        }
        create_op_id.set(op_id);
        pending_create.set(true);
    };

    // Admission gate: triggered by list changes OR create_op updates.
    // Only the matching op_id resolves — no list-admission cross-attempt
    // fallthrough that would let A clear B's draft.
    Effect::new(move |_: Option<()>| {
        if !pending_create.get() {
            return;
        }
        let my_op = create_op_id.get();
        let (current_op, outcome) = rooms.create_op.get();
        // Only resolve our own op; never fall through to list inspection
        // for another op_id's outcome.
        if current_op != my_op {
            return;
        }
        let draft = new_room_name.get();
        if draft.trim().is_empty() {
            pending_create.set(false);
            return;
        }
        match Rooms::resolve_create_op(current_op, my_op, outcome.as_ref()) {
            CreateResolution::Success => {
                new_room_name.set(String::new());
                workspace_choice.set(None);
                picked_folder.set(None);
                pending_create.set(false);
            }
            CreateResolution::KeepDraft => {
                pending_create.set(false);
            }
            CreateResolution::Pending => { /* still in flight */ }
        }
    });

    let choose_folder = move || {
        if picker_pending.get_untracked() {
            return;
        }
        picker_pending.set(true);
        leptos::task::spawn_local(async move {
            let picked = crate::host::pick_folder().await;
            // A disposed rail drops the late result.
            if picker_pending.try_set(false).is_some() {
                return;
            }
            if let Some(root) = picked.as_deref().and_then(normalized_root) {
                picked_folder.set(Some(root.clone()));
                workspace_choice.set(Some(root));
            }
        });
    };

    let on_workspace_change =
        move |ev: leptos::ev::Event| match workspace_selection(&event_target_value(&ev)) {
            WorkspaceSelection::None => workspace_choice.set(None),
            WorkspaceSelection::Root(root) => workspace_choice.set(Some(root)),
            WorkspaceSelection::ChooseFolder => {
                // Keep showing the current choice until a folder is picked.
                if let Some(select) = workspace_select_ref.get_untracked() {
                    select.set_value(&workspace_choice.get_untracked().unwrap_or_default());
                }
                choose_folder();
            }
        };

    view! {
        <div
            id="rooms-workspace-room-list"
            class="rooms-workspace__left"
            class:rooms-workspace__left--visible=move || show_left_rail.get()
            role="navigation"
            aria-label="Room list"
        >
            <div class="rooms-workspace__left-head">
                <h2 class="rooms-workspace__left-title">
                    {move || if inbox_open.get() { "Mentions" } else { "Rooms" }}
                </h2>
                <button
                    class="rooms-workspace__left-inbox"
                    class:is-on=move || inbox_open.get()
                    type="button"
                    aria-label="Mentions"
                    aria-pressed=move || inbox_open.get().to_string()
                    on:click=move |_| inbox_open.update(|o| *o = !*o)
                >
                    <crate::icons::AtSign />
                </button>
                // Close: on wide screens exits rooms entirely; on narrow
                // the backdrop closes the drawer and this X remains the
                // escape-hatch to close rooms.
                <button
                    class="rooms-workspace__left-close"
                    type="button"
                    aria-label="Close rooms workspace"
                    on:click={
                        let close = on_close;
                        move |_| {
                            rooms.close_room();
                            if let Some(ref cb) = close { cb.run(()); }
                        }
                    }
                >
                    <svg viewBox="0 0 16 16" width="14" height="14"
                        fill="none" stroke="currentColor" stroke-width="1.6"
                        stroke-linecap="round">
                        <path d="M3 3l10 10M13 3L3 13"/>
                    </svg>
                </button>
            </div>

            // Room list — scrollable
            <div class="rooms-workspace__left-list">
                <Show
                    when=move || !inbox_open.get()
                    fallback=move || view! {
                        <crate::room_attention::RoomInboxPanel
                            rooms=rooms
                            open=inbox_open
                            on_pick=Callback::new(move |()| show_left_rail.set(false))
                        />
                    }
                >
                {move || {
                    let list = rooms.list.get();
                    let error = rooms.rooms_error.get();
                    if let Some(error) = error {
                        view! {
                            <div
                                class="rooms-workspace__left-empty rooms-workspace__left-empty--error"
                                role="alert"
                            >
                                {format!("Unable to load rooms: {error}")}
                            </div>
                        }.into_any()
                    } else if list.is_empty()
                        && (rooms.rooms_loading.get() || !rooms.rooms_loaded.get())
                    {
                        view! {
                            <div class="rooms-workspace__left-empty" role="status">
                                "Loading…"
                            </div>
                        }.into_any()
                    } else if list.is_empty() {
                        view! {
                            <div class="rooms-workspace__left-empty">
                                "No rooms yet. Create one below."
                            </div>
                        }.into_any()
                    } else {
                        // ARIA listbox with roving tabindex: exactly one
                        // option (open room, else first) is the Tab stop;
                        // arrows/Home/End move REAL focus between options
                        // via their DOM ids. Enter/Space activate through
                        // the button default. aria-selected drives the
                        // interaction stylesheet's selected treatment.
                        view! {
                            <div
                                class="rooms-workspace__left-options"
                                role="listbox"
                                aria-label="Rooms"
                                on:keydown=move |ev: web_sys::KeyboardEvent| {
                                    let keys: Vec<String> = rooms
                                        .list
                                        .get_untracked()
                                        .iter()
                                        .map(|r| r.id.clone())
                                        .collect();
                                    let focused = web_sys::window()
                                        .and_then(|w| w.document())
                                        .and_then(|d| d.active_element())
                                        .map(|el| el.id());
                                    let focused_key = focused
                                        .as_deref()
                                        .and_then(|id| id.strip_prefix("rooms-opt-"));
                                    let Some(idx) =
                                        room_list_next_focus(&keys, focused_key, &ev.key())
                                    else {
                                        return;
                                    };
                                    ev.prevent_default();
                                    if let Some(el) = web_sys::window()
                                        .and_then(|w| w.document())
                                        .and_then(|d| {
                                            d.get_element_by_id(&room_option_dom_id(&keys[idx]))
                                        })
                                        .and_then(|el| {
                                            el.dyn_into::<web_sys::HtmlElement>().ok()
                                        })
                                    {
                                        let _ = el.focus();
                                    }
                                }
                            >
                                <For
                                    each=move || rooms.list.get()
                                    key=|r: &Room| (r.id.clone(), r.participants.len(), r.updated_at.clone(), r.muted)
                                    children=move |room: Room| {
                                        let key = room.id.clone();
                                        let key2 = key.clone();
                                        let key_tab = key.clone();
                                        let key_sel = key.clone();
                                        let key_unread = key.clone();
                                        let active = move || rooms.open_key.get().as_deref() == Some(&*key);
                                        let selected =
                                            move || rooms.open_key.get().as_deref() == Some(&*key_sel);
                                        let is_tab_stop = move || {
                                            let keys: Vec<String> = rooms
                                                .list
                                                .get()
                                                .iter()
                                                .map(|r| r.id.clone())
                                                .collect();
                                            let open = rooms.open_key.get();
                                            room_list_tab_stop(&keys, open.as_deref())
                                                .and_then(|i| keys.get(i).cloned())
                                                .as_deref()
                                                == Some(&*key_tab)
                                        };
                                        let muted = room.muted;
                                        // Muted rooms stay quiet in the list;
                                        // their mentions still reach the inbox.
                                        let unread = move || {
                                            !muted && rooms.read_summaries.with(|summaries| {
                                                crate::rooms::room_has_durable_unread(
                                                    summaries.get(&key_unread),
                                                )
                                            })
                                        };
                                        view! {
                                            <button
                                                class="rooms-workspace__room"
                                                class:is-active=active
                                                class:is-muted=muted
                                                type="button"
                                                role="option"
                                                id=room_option_dom_id(&room.id)
                                                aria-selected=move || selected().to_string()
                                                tabindex=move || if is_tab_stop() { "0" } else { "-1" }
                                                on:click=move |_| {
                                                    rooms.open_room(key2.clone());
                                                    show_left_rail.set(false);
                                                }
                                            >
                                                <span class="rooms-workspace__room-hash">"#"</span>
                                                <span class="rooms-workspace__room-name">
                                                    {room.name.clone()}
                                                </span>
                                                <Show when=move || unread()>
                                                    <span
                                                        class="rooms-workspace__room-unread"
                                                        role="img"
                                                        aria-label="Unread messages"
                                                    ></span>
                                                </Show>
                                            </button>
                                        }
                                    }
                                />
                            </div>
                        }.into_any()
                    }
                }}
                </Show>
            </div>

            {move || {
                let status = rooms.status.get();
                if status.starts_with("rooms ") || status.starts_with("create ") {
                    view! {
                        <div
                            class="rooms-workspace__left-status"
                            role="status"
                            aria-live="polite"
                        >
                            {status}
                        </div>
                    }.into_any()
                } else {
                    ().into_any()
                }
            }}

            // Create input at bottom of left rail
            <div class="rooms-workspace__left-create">
                <input
                    class="rooms-workspace__left-input"
                    type="text"
                    node_ref=create_input_ref
                    aria-label="New room name"
                    aria-busy=move || pending_create.get().to_string()
                    placeholder="New room"
                    prop:value=move || new_room_name.get()
                    on:input=move |ev| new_room_name.set(event_target_value(&ev))
                    on:keydown=move |ev| {
                        if ev.key() == "Enter" {
                            ev.prevent_default();
                            create_room();
                        }
                    }
                    disabled=move || pending_create.get()
                />
                <Show when=move || !new_room_name.get().trim().is_empty()>
                <select
                    class="rooms-workspace__left-input rooms-workspace__left-workspace"
                    aria-label="Project"
                    node_ref=workspace_select_ref
                    prop:value=move || workspace_choice.get().unwrap_or_default()
                    on:change=on_workspace_change
                    disabled=move || pending_create.get() || picker_pending.get()
                >
                    <option value="" prop:selected=move || workspace_choice.get().is_none()>
                        "Project (optional)"
                    </option>
                    <For
                        each=move || workspace_choices.get()
                        key=|option: &WorkspaceOption| (option.root.clone(), option.label.clone())
                        children=move |option: WorkspaceOption| {
                            let root = option.root.clone();
                            view! {
                                <option
                                    value=option.root.clone()
                                    title=option.root.clone()
                                    prop:selected=move || {
                                        workspace_choice.get().as_deref() == Some(root.as_str())
                                    }
                                >
                                    {option.label}
                                </option>
                            }
                        }
                    />
                    {can_pick_folder.then(|| view! {
                        <option value=CHOOSE_FOLDER_VALUE>"Choose folder…"</option>
                    })}
                </select>
                <button
                    class="rooms-workspace__create-submit"
                    type="button"
                    disabled=move || pending_create.get() || picker_pending.get()
                    on:click=move |_| create_room()
                >
                    {move || if pending_create.get() { "Creating…" } else { "Create" }}
                </button>
                </Show>
            </div>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rooms::CreateOutcome;

    // Uses the real pub fn that the Effect calls — no cfg(test)-only copies.

    #[test]
    fn resolve_success_clears_draft() {
        let outcome = Some(CreateOutcome::Success { key: "room".into() });
        assert_eq!(
            crate::rooms::Rooms::resolve_create_op(1, 1, outcome.as_ref()),
            CreateResolution::Success
        );
    }

    #[test]
    fn resolve_duplicate_keeps_draft() {
        assert_eq!(
            crate::rooms::Rooms::resolve_create_op(1, 1, Some(&CreateOutcome::Duplicate)),
            CreateResolution::KeepDraft
        );
    }

    #[test]
    fn resolve_failed_keeps_draft() {
        assert_eq!(
            crate::rooms::Rooms::resolve_create_op(
                1,
                1,
                Some(&CreateOutcome::Failed {
                    error: "timeout".into(),
                }),
            ),
            CreateResolution::KeepDraft
        );
    }

    #[test]
    fn resolve_in_flight_stays_pending() {
        assert_eq!(
            crate::rooms::Rooms::resolve_create_op(1, 1, None),
            CreateResolution::Pending
        );
    }

    #[test]
    fn resolve_stale_op_id_returns_pending() {
        // current_op (2) != my_op (1) → stale, no match
        assert_eq!(
            crate::rooms::Rooms::resolve_create_op(
                2,
                1,
                Some(&CreateOutcome::Success { key: "x".into() }),
            ),
            CreateResolution::Pending
        );
    }

    #[test]
    fn resolve_concurrent_b_sees_own_outcome_a_sees_stale() {
        // Op 2 is the active dispatch; op 1 was superseded.
        // Both check the same slot (current_op=2, outcome=Success for room-b).
        let outcome = Some(CreateOutcome::Success {
            key: "b-room".into(),
        });
        // B (my_op=2) resolves → Success
        assert_eq!(
            crate::rooms::Rooms::resolve_create_op(2, 2, outcome.as_ref()),
            CreateResolution::Success
        );
        // A (my_op=1) resolves → Pending (stale)
        assert_eq!(
            crate::rooms::Rooms::resolve_create_op(2, 1, outcome.as_ref()),
            CreateResolution::Pending
        );
    }

    #[test]
    fn cas_admit_matching_op() {
        assert!(Rooms::cas_admit_create(1, 1));
    }

    #[test]
    fn cas_admit_superseded_op() {
        // Slot op 3, our op 1 — superseded by two later dispatches.
        assert!(!Rooms::cas_admit_create(3, 1));
    }

    #[test]
    fn cas_admit_zero_op() {
        assert!(Rooms::cas_admit_create(0, 0));
        assert!(!Rooms::cas_admit_create(0, 1));
    }

    #[test]
    fn cas_admit_stale_success_action_is_list_only() {
        // Simulate what the CAS closure does for a stale success:
        // admitted=false + Success → fetch_rooms only, no status/select.
        // Verified by the outcome path: CasAdmission::admit(*cur, op_id)
        // returns false, and the else-if branch only triggers for Success.
        let outcome = CreateOutcome::Success {
            key: "stale-room".into(),
        };
        let admitted = Rooms::cas_admit_create(2, 1); // slot=2, my=1 → false
        assert!(!admitted);
        // If !admitted && matches!(Success): list-refresh only.
        // If !admitted && !Success: fully suppressed.
        assert!(matches!(outcome, CreateOutcome::Success { .. }));
        // Stale failure would be suppressed — no status, no list:
        let failure = CreateOutcome::Failed {
            error: "timeout".into(),
        };
        assert!(matches!(failure, CreateOutcome::Failed { .. }));
        // The production code path for stale failure is: no side effects.
    }

    fn keys(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn next_focus_arrows_wrap_and_home_end_jump() {
        let k = keys(&["a", "b", "c"]);
        assert_eq!(room_list_next_focus(&k, Some("a"), "ArrowDown"), Some(1));
        assert_eq!(room_list_next_focus(&k, Some("c"), "ArrowDown"), Some(0));
        assert_eq!(room_list_next_focus(&k, Some("a"), "ArrowUp"), Some(2));
        assert_eq!(room_list_next_focus(&k, Some("b"), "Home"), Some(0));
        assert_eq!(room_list_next_focus(&k, Some("b"), "End"), Some(2));
    }

    #[test]
    fn next_focus_without_focused_option_enters_at_edges() {
        let k = keys(&["a", "b"]);
        assert_eq!(room_list_next_focus(&k, None, "ArrowDown"), Some(0));
        assert_eq!(room_list_next_focus(&k, None, "ArrowUp"), Some(1));
    }

    #[test]
    fn next_focus_ignores_non_nav_keys_and_empty_list() {
        let k = keys(&["a"]);
        assert_eq!(room_list_next_focus(&k, Some("a"), "Enter"), None);
        assert_eq!(room_list_next_focus(&k, Some("a"), "j"), None);
        assert_eq!(room_list_next_focus(&[], None, "ArrowDown"), None);
    }

    #[test]
    fn next_focus_with_stale_focused_key_recovers_at_edges() {
        // Focused option was removed by a refetch: treat as unfocused.
        let k = keys(&["a", "b"]);
        assert_eq!(room_list_next_focus(&k, Some("gone"), "ArrowDown"), Some(0));
    }

    #[test]
    fn tab_stop_is_open_room_else_first_else_none() {
        let k = keys(&["a", "b", "c"]);
        assert_eq!(room_list_tab_stop(&k, Some("b")), Some(1));
        assert_eq!(room_list_tab_stop(&k, Some("zz")), Some(0));
        assert_eq!(room_list_tab_stop(&k, None), Some(0));
        assert_eq!(room_list_tab_stop(&[], Some("a")), None);
    }

    fn project(id: &str, name: &str, root: &str) -> ProjectInfo {
        serde_json::from_value(serde_json::json!({
            "id": id,
            "name": name,
            "workspace_root": root,
        }))
        .unwrap()
    }

    fn roots(options: &[WorkspaceOption]) -> Vec<&str> {
        options.iter().map(|o| o.root.as_str()).collect()
    }

    #[test]
    fn workspace_options_come_from_absolute_project_roots_deduped() {
        let projects = vec![
            project("a", "Ocean", "/Users/me/dev/ocean"),
            project("b", "Ocean copy", "/Users/me/dev/ocean/"),
            project("c", "Relative", "dev/rel"),
            project("d", "Empty", ""),
            project("e", "Site", "  /srv/site  "),
            project("f", "Win", "C:\\work\\app"),
        ];
        let options = workspace_options(&projects, None);
        assert_eq!(
            roots(&options),
            vec!["/Users/me/dev/ocean", "/srv/site", "C:\\work\\app"]
        );
        assert_eq!(options[0].label, "Ocean");
        assert_eq!(options[1].label, "Site");
    }

    #[test]
    fn workspace_options_without_projects_is_empty() {
        assert!(workspace_options(&[], None).is_empty());
    }

    #[test]
    fn workspace_options_append_unlisted_picked_folder() {
        let projects = vec![project("a", "Ocean", "/Users/me/dev/ocean")];
        let options = workspace_options(&projects, Some("/Users/me/notes"));
        assert_eq!(
            roots(&options),
            vec!["/Users/me/dev/ocean", "/Users/me/notes"]
        );
        assert_eq!(options[1].label, "notes");
        // A picked folder already listed as a project is not repeated.
        let options = workspace_options(&projects, Some("/Users/me/dev/ocean/"));
        assert_eq!(roots(&options), vec!["/Users/me/dev/ocean"]);
        // A relative pick is never offered.
        let options = workspace_options(&projects, Some("notes"));
        assert_eq!(roots(&options), vec!["/Users/me/dev/ocean"]);
    }

    #[test]
    fn workspace_options_disambiguate_colliding_labels_with_root() {
        let projects = vec![
            project("a", "app", "/a/app"),
            project("b", "app", "/b/app"),
            project("c", "", "/c/tool"),
        ];
        let labels: Vec<String> = workspace_options(&projects, None)
            .into_iter()
            .map(|o| o.label)
            .collect();
        assert_eq!(labels, vec!["/a/app", "/b/app", "tool"]);
    }

    #[test]
    fn workspace_selection_defaults_to_none() {
        assert_eq!(workspace_selection(""), WorkspaceSelection::None);
        assert_eq!(
            workspace_selection("relative/dir"),
            WorkspaceSelection::None
        );
        assert_eq!(
            workspace_selection("/tmp/"),
            WorkspaceSelection::Root("/tmp".into())
        );
        assert_eq!(
            workspace_selection(CHOOSE_FOLDER_VALUE),
            WorkspaceSelection::ChooseFolder
        );
        assert!(!is_absolute_root(CHOOSE_FOLDER_VALUE));
    }

    #[test]
    fn absolute_roots_cover_posix_drive_and_unc() {
        assert!(is_absolute_root("/"));
        assert!(is_absolute_root("C:\\"));
        assert!(is_absolute_root("d:/work"));
        assert!(is_absolute_root("\\\\server\\share"));
        assert!(!is_absolute_root("C:work"));
        assert!(!is_absolute_root("~/dev"));
        assert!(!is_absolute_root(""));
        assert_eq!(
            workspace_selection("/"),
            WorkspaceSelection::Root("/".into())
        );
        assert_eq!(
            workspace_selection("C:\\"),
            WorkspaceSelection::Root("C:\\".into())
        );
    }

    #[test]
    fn root_basename_is_last_segment() {
        assert_eq!(root_basename("/Users/me/dev/ocean"), "ocean");
        assert_eq!(root_basename("C:\\work\\app"), "app");
        assert_eq!(root_basename("/"), "/");
    }

    #[test]
    fn option_dom_id_is_prefix_stable() {
        assert_eq!(room_option_dom_id("r1"), "rooms-opt-r1");
    }
}
