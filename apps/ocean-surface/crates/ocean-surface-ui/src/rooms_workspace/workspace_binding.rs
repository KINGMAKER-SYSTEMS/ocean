//! The room details drawer's "Default folder" row (design
//! `OCEAN_ROOMS_DESIGN_DIRECTION.md` §5.2): the open room's `workspace_root`
//! in mono (or "none"), the cwd rule in one sentence, and a ghost Edit that
//! reveals the shared directory picker with Bind / Unbind. Bind and Unbind
//! are the only callers of `PATCH /v1/rooms/persistent/{key}` on this
//! surface ([`Rooms::set_open_room_workspace`]): a string rebinds, an
//! explicit `null` unbinds.
//!
//! Contract boundaries kept from the legacy port (ocean-surface#240): the
//! folder is on the DAEMON's host and the daemon alone accepts or refuses it
//! (`400 invalid_workspace_root` renders beside the control, draft kept);
//! nothing here validates a path in the browser, and nothing here softens or
//! restates what an unbound room can run — the daemon's cwd rule is agent
//! folder, then room folder grant, then this default, and a turn it cannot
//! place is refused at the action (`workspace_unavailable`).

use leptos::prelude::*;

use super::access::local_store_write_gate;
use super::working_directory::WorkingDirectoryPicker;
use crate::daemon::ProjectInfo;
use crate::rooms::Rooms;

/// The row's value: the bound root verbatim, or "none".
pub(super) fn default_folder_label(root: Option<&str>) -> String {
    match root.map(str::trim).filter(|root| !root.is_empty()) {
        Some(root) => root.to_string(),
        None => "none".to_string(),
    }
}

/// Bind sends only a real change: a non-blank draft that differs from the
/// stored root, with no PATCH in flight. Whitespace is the one thing judged
/// here; the path itself is the daemon's.
pub(super) fn bind_enabled(draft: &str, current: Option<&str>, in_flight: bool) -> bool {
    let draft = draft.trim();
    !in_flight && !draft.is_empty() && Some(draft) != current
}

/// Unbind exists only while a root is bound; a second unbind has nothing to
/// send.
pub(super) fn unbind_offered(current: Option<&str>, in_flight: bool) -> bool {
    !in_flight && current.is_some_and(|root| !root.trim().is_empty())
}

/// What the picker should hold when the open room changes identity or the
/// daemon answers a new binding: the stored root, so a rebind is an edit
/// rather than a retype. A roster write that keeps both unchanged must not
/// reseed (it would wipe a path mid-type).
pub(super) fn reseed_needed(
    previous: Option<&(String, Option<String>)>,
    current: &(String, Option<String>),
) -> bool {
    previous != Some(current)
}

/// The drawer's "Default folder" section for the open room.
#[component]
pub(super) fn DefaultFolderSection(
    rooms: Rooms,
    /// The daemon's project catalogue, shared with the create form.
    projects: Signal<Vec<ProjectInfo>>,
) -> impl IntoView {
    let editing = RwSignal::new(false);
    let draft = RwSignal::new(String::new());
    // Bumped on every reseed so the picker drops manual mode with the draft
    // it belonged to (another room may be bound to a catalogue row).
    let picker_reset = RwSignal::new(0u32);
    let current_root = Memo::new(move |_| {
        rooms
            .open_room
            .get()
            .and_then(|room| room.workspace_root)
            .filter(|root| !root.trim().is_empty())
    });
    // Seed from the stored binding when the room or its binding changes —
    // never on an unrelated roster frame — and leave edit mode, because the
    // daemon's answer is now the row.
    Effect::new(move |previous: Option<Option<(String, Option<String>)>>| {
        let identity = rooms
            .open_room
            .get()
            .map(|room| (room.id, room.workspace_root));
        let previous = previous.flatten();
        if let Some(current) = identity.as_ref() {
            if reseed_needed(previous.as_ref(), current) {
                draft.set(current.1.clone().unwrap_or_default());
                picker_reset.update(|n| *n = n.wrapping_add(1));
                editing.set(false);
            }
        }
        identity
    });
    let in_flight = move || rooms.workspace_patch_in_flight.get();
    let can_edit = move || local_store_write_gate(rooms.access.get().as_ref());
    let bind = move || {
        let root = draft.get_untracked();
        if !bind_enabled(
            &root,
            current_root.get_untracked().as_deref(),
            in_flight(),
        ) {
            return;
        }
        rooms.set_open_room_workspace(Some(root));
    };
    let unbind = move || {
        if !unbind_offered(current_root.get_untracked().as_deref(), in_flight()) {
            return;
        }
        rooms.set_open_room_workspace(None);
    };
    let cancel = move || {
        draft.set(current_root.get_untracked().unwrap_or_default());
        picker_reset.update(|n| *n = n.wrapping_add(1));
        rooms.workspace_patch_error.set(None);
        editing.set(false);
    };

    view! {
        <div class="rooms-workspace__right-section-title">"Default folder"</div>
        <div class="rooms-workspace__workspace" role="group" aria-label="Default folder">
            <div class="rooms-workspace__workspace-row">
                <code
                    class="rooms-workspace__workspace-root"
                    class:rooms-workspace__workspace-root--none=move || current_root.get().is_none()
                    title=move || current_root.get().unwrap_or_default()
                >
                    {move || default_folder_label(current_root.get().as_deref())}
                </code>
                // Ghost Edit, absent (not disabled) when this host may not
                // write to the room's local store.
                {move || can_edit().then(|| view! {
                    <button
                        class="rooms-workspace__workspace-edit"
                        type="button"
                        aria-expanded=move || editing.get().to_string()
                        aria-controls="rooms-details-workspace-controls"
                        on:click=move |_| {
                            if editing.get_untracked() {
                                cancel();
                            } else {
                                editing.set(true);
                            }
                        }
                    >
                        {move || if editing.get() { "Cancel" } else { "Edit" }}
                    </button>
                })}
            </div>
            <span class="rooms-workspace__workspace-help">
                "Used only when an agent has no shared folder of its own. Resolved on \
                 the machine running the daemon, not in this browser."
            </span>
            <Show when=move || editing.get() && can_edit()>
                <div
                    class="rooms-workspace__workspace-controls"
                    id="rooms-details-workspace-controls"
                >
                    // Paths are folders on the DAEMON's host; the catalogue is
                    // `daemon.projects`, never a browser filesystem API, and
                    // only the daemon validates what is chosen or typed.
                    <WorkingDirectoryPicker
                        projects=projects
                        value=draft
                        disabled=Signal::derive(in_flight)
                        reset=picker_reset
                        id_base="rooms-details-workspace"
                        label="Workspace folder on the daemon host"
                        help="Choose a project or worktree registered on the daemon's machine, or enter an absolute path there."
                        allow_empty=false
                        on_enter=Callback::new(move |()| bind())
                    />
                    {move || rooms.workspace_patch_error.get().map(|error| view! {
                        <div class="rooms-workspace__workspace-error" role="alert">{error}</div>
                    })}
                    <div class="rooms-workspace__workspace-actions">
                        <button
                            class="rooms-workspace__workspace-bind"
                            type="button"
                            aria-busy=move || in_flight().to_string()
                            disabled=move || {
                                !bind_enabled(
                                    &draft.get(),
                                    current_root.get().as_deref(),
                                    in_flight(),
                                )
                            }
                            on:click=move |_| bind()
                        >
                            {move || if in_flight() { "Saving\u{2026}" } else { "Bind" }}
                        </button>
                        {move || unbind_offered(current_root.get().as_deref(), false).then(|| view! {
                            <button
                                class="rooms-workspace__workspace-unbind"
                                type="button"
                                disabled=move || in_flight()
                                on:click=move |_| unbind()
                            >
                                "Unbind"
                            </button>
                        })}
                    </div>
                </div>
            </Show>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_folder_label_is_the_root_or_none() {
        assert_eq!(default_folder_label(Some("/srv/ocean")), "/srv/ocean");
        assert_eq!(default_folder_label(Some("  ")), "none");
        assert_eq!(default_folder_label(None), "none");
    }

    #[test]
    fn bind_sends_only_a_real_change_and_never_while_in_flight() {
        assert!(bind_enabled("/srv/ocean", None, false));
        assert!(bind_enabled("/srv/other", Some("/srv/ocean"), false));
        // Same root as stored: nothing to send.
        assert!(!bind_enabled("/srv/ocean", Some("/srv/ocean"), false));
        assert!(!bind_enabled("  /srv/ocean  ", Some("/srv/ocean"), false));
        // Blank is the unbind body; the Unbind control owns that intent.
        assert!(!bind_enabled("", None, false));
        assert!(!bind_enabled("   ", Some("/srv/ocean"), false));
        assert!(!bind_enabled("/srv/other", Some("/srv/ocean"), true));
        // No path judgement here: a relative draft is sent and the daemon
        // answers `invalid_workspace_root`.
        assert!(bind_enabled("relative/dir", None, false));
    }

    #[test]
    fn unbind_is_offered_only_while_bound_and_idle() {
        assert!(unbind_offered(Some("/srv/ocean"), false));
        assert!(!unbind_offered(Some("/srv/ocean"), true));
        assert!(!unbind_offered(None, false));
        assert!(!unbind_offered(Some(""), false));
    }

    #[test]
    fn reseed_follows_room_identity_and_binding_but_not_roster_frames() {
        let bound = ("ops".to_string(), Some("/srv/ocean".to_string()));
        let rebound = ("ops".to_string(), Some("/srv/other".to_string()));
        let unbound = ("ops".to_string(), None);
        let other = ("campaigns".to_string(), Some("/srv/ocean".to_string()));
        assert!(reseed_needed(None, &bound));
        // A roster SSE frame rewrites `open_room` with the same id and root.
        assert!(!reseed_needed(Some(&bound), &bound));
        assert!(reseed_needed(Some(&bound), &rebound));
        assert!(reseed_needed(Some(&bound), &unbound));
        assert!(reseed_needed(Some(&bound), &other));
    }
}
