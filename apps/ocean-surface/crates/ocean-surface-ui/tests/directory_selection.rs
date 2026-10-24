//! Guards the room working-directory chooser as a daemon-catalogue projection
//! (ported from Risingtides-dev/ocean-surface#240 `tests/directory_selection.rs`
//! onto the split Rooms module; design `OCEAN_ROOMS_DESIGN_DIRECTION.md` §5.2).
//!
//! This crate is binary-only, so integration guards inspect release source.
//! Pure option flattening and the selection decision are covered beside the
//! production helpers in `rooms_workspace/working_directory.rs`.

use std::path::Path;

const WORKSPACE: &str = "src/rooms_workspace/mod.rs";
const ROOM_LIST: &str = "src/rooms_workspace/room_list.rs";
const BINDING: &str = "src/rooms_workspace/workspace_binding.rs";
const PICKER: &str = "src/rooms_workspace/working_directory.rs";
const ROOMS: &str = "src/rooms.rs";

fn read(rel: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

/// The production half of a module (above its `#[cfg(test)] mod tests`).
fn production(rel: &str) -> String {
    let source = read(rel);
    source
        .rsplit_once("#[cfg(test)]\nmod tests {")
        .map_or(source.clone(), |(production, _)| production.to_string())
}

fn without_whitespace(input: &str) -> String {
    input.chars().filter(|c| !c.is_whitespace()).collect()
}

/// Code lines only: `//` comments may NAME what the daemon does.
fn code_only(source: &str) -> String {
    source
        .lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Lowercased, whitespace-free, string-continuation backslashes gone, so a
/// banned phrase cannot hide behind a line wrap or capitalisation.
fn normalize(source: &str) -> String {
    without_whitespace(source).replace('\\', "").to_lowercase()
}

#[test]
fn both_mounts_share_the_daemon_project_signal_without_fetching_again() {
    let workspace = without_whitespace(&production(WORKSPACE));
    assert!(
        workspace.contains(
            "letprojects:Signal<Vec<crate::daemon::ProjectInfo>>=daemon.projects.into();"
        ),
        "RoomsWorkspace reads the daemon-owned project catalogue once",
    );
    assert!(
        workspace.contains("<RoomListRailrooms=roomsprojects=projects"),
        "the create form consumes the shared catalogue",
    );
    assert!(
        workspace.contains("<DefaultFolderSectionrooms=roomsprojects=projects/>"),
        "the drawer's default-folder row consumes the same catalogue",
    );
    let picker = production(PICKER);
    assert!(
        !picker.contains("Request::")
            && !picker.contains("/v1/projects")
            && !picker.contains("fetch_projects"),
        "the working-directory control must not issue a second project request",
    );
}

#[test]
fn create_and_default_folder_use_the_one_picker_and_the_existing_room_routes() {
    let room_list = without_whitespace(&production(ROOM_LIST));
    let binding = without_whitespace(&production(BINDING));
    assert_eq!(room_list.matches("<WorkingDirectoryPicker").count(), 1);
    assert_eq!(binding.matches("<WorkingDirectoryPicker").count(), 1);
    // No third mount anywhere else in the page.
    let src_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut mounts = 0usize;
    let mut stack = vec![src_dir];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).expect("read_dir") {
            let path = entry.expect("entry").path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
                mounts += std::fs::read_to_string(&path)
                    .unwrap_or_default()
                    .matches("<WorkingDirectoryPicker")
                    .count();
            }
        }
    }
    assert_eq!(
        mounts, 2,
        "room creation and the default-folder row share one directory control"
    );
    assert!(
        room_list.contains("projects=projectsvalue=create_workspace"),
        "create consumes the shared catalogue and retains its create draft",
    );
    assert!(
        binding.contains("projects=projectsvalue=draft"),
        "the default-folder row consumes the shared catalogue and the retained binding draft",
    );
    assert!(
        room_list.contains(
            "letworkspace_root=create_workspace_root(&create_workspace.get_untracked());"
        ) && room_list.contains("rooms.create_room(name.clone(),workspace_root);"),
        "create still sends the existing optional workspace_root field through Rooms::create_room",
    );
    assert!(
        binding.contains("rooms.set_open_room_workspace(Some(root));")
            && binding.contains("rooms.set_open_room_workspace(None);"),
        "bind and unbind go through the guarded workspace PATCH dispatcher",
    );
    // Create is `allow_empty=true` (an unbound room is a real choice); the
    // bind control is `allow_empty=false` (blank is Unbind's job).
    assert!(room_list.contains("allow_empty=true"));
    assert!(binding.contains("allow_empty=false"));
}

#[test]
fn picker_offers_registered_roots_a_native_chooser_and_an_explicit_manual_path() {
    let picker = without_whitespace(&production(PICKER));
    assert!(
        picker.contains("forprojectinprojects")
            && picker.contains("forworktreein&project.worktrees"),
        "the picker must include both registered project and worktree roots",
    );
    assert!(
        picker.contains("\"Otherabsolutepath\\u{2026}\"")
            && picker.contains("value.set(event_target_value(&ev));"),
        "a manual absolute-path fallback must remain available without a browser filesystem API",
    );
    assert!(
        picker.contains("DirectoryChoice::Manual")
            && picker.contains("}elseifdirectory_is_registered(projects,value){"),
        "a stored room binding absent from the catalogue must remain visible and editable",
    );
    // The desktop shell's chooser goes through the host seam and only there.
    assert!(picker.contains("letcan_pick_folder=crate::host::running_in_tauri();"));
    assert!(picker.contains("letpicked=crate::host::pick_folder().await;"));
    assert!(picker.contains("{can_pick_folder.then(||view!{<optionvalue=CHOOSE_FOLDER_CHOICE>"));
    assert!(
        !picker.contains("showDirectoryPicker")
            && !picker.contains("webkitdirectory")
            && !picker.contains("type=\"file\""),
        "browser filesystem APIs are not authority for the daemon host",
    );
}

/// The browser cannot see the daemon's filesystem, so nothing in the picker or
/// the default-folder row judges a path: no absoluteness, existence or
/// canonical-form check. The daemon's `400 invalid_workspace_root` is the only
/// verdict, rendered as a sentence by `rooms.rs`.
#[test]
fn neither_the_picker_nor_the_folder_row_prevalidates_a_path_in_the_browser() {
    for module in [PICKER, BINDING] {
        let code = code_only(&production(module));
        for forbidden in [
            "starts_with(",
            "is_absolute",
            "Path::",
            "PathBuf",
            "canonicalize",
            "exists(",
            "invalid_workspace_root",
            "normalized_root",
            "is_absolute_root",
        ] {
            assert!(
                !code.contains(forbidden),
                "`{forbidden}` in {module} would judge a daemon-host path in the browser; \
                 only the daemon validates it",
            );
        }
    }
    // The create form sends the draft trimmed, nothing more.
    let room_list = without_whitespace(&production(ROOM_LIST));
    assert!(room_list.contains("pub(super)fncreate_workspace_root(draft:&str)->Option<String>{letroot=draft.trim();(!root.is_empty()).then(||root.to_string())}"));
    // And the refusal has one sentence, with the draft kept.
    let rooms = production(ROOMS);
    assert!(rooms.contains("(_, Some(\"invalid_workspace_root\")) => {\n            \"workspace folder not found on this Ocean\".to_string()\n        }"));
}

/// Contract boundaries (ocean-surface#240, kept verbatim): the folder is on
/// the DAEMON's host, and nothing softens an unbound room with a grant story.
/// Design ruling (§5.2, S7): there is no room-wide "cannot run" warning either
/// — the only cannot-run copy is per agent, and it is not built here.
#[test]
fn copy_names_the_daemon_host_and_neither_softens_nor_warns_room_wide() {
    let picker_code = normalize(&code_only(&production(PICKER)));
    let binding_code = normalize(&code_only(&production(BINDING)));
    let room_list_code = normalize(&code_only(&production(ROOM_LIST)));
    assert!(
        room_list_code.contains(&normalize("registered on the daemon's machine")),
        "the create picker's help must name the daemon's machine as the owner of the path",
    );
    assert!(
        binding_code.contains(&normalize("registered on the daemon's machine")),
        "the bind picker's help must name the daemon's machine as the owner of the path",
    );
    assert!(
        binding_code.contains(&normalize(
            "Resolved on the machine running the daemon, not in this browser"
        )),
        "the default-folder row must say whose machine resolves the path",
    );
    assert!(
        binding_code.contains(&normalize("Used only when an agent has no shared folder")),
        "the default-folder row states the daemon's cwd rule in one sentence (design §5.2)",
    );
    assert!(
        picker_code.contains(&normalize("Other absolute path on the daemon host")),
        "the manual field's accessible name must say the path is on the daemon host",
    );
    let sources = [
        (PICKER, picker_code),
        (BINDING, binding_code),
        (ROOM_LIST, room_list_code),
        (ROOMS, normalize(&code_only(&production(ROOMS)))),
    ];
    for banned in [
        "resource grants may still supply",
        "resource grants may supply",
        "grants may still supply",
        "may still supply a working directory",
        "No room default",
        "Use as room default",
        "Clear default",
        "explicit room default",
        "connected machine",
    ] {
        let needle = normalize(banned);
        for (file, text) in &sources {
            assert!(
                !text.contains(&needle),
                "`{banned}` in {file} softens or rewrites the unbound-room contract",
            );
        }
    }
    for (file, text) in &sources[..2] {
        assert!(
            !text.contains(&normalize("resource grants")),
            "{file} must not describe a resource-grant fallback for an unbound room",
        );
        assert!(
            !text.contains(&normalize("cannot run"))
                && !text.contains(&normalize("Agents in this room")),
            "{file} must not carry a room-wide cannot-run warning (design §5.2: per-agent only)",
        );
    }
}

/// Help text is linked to both controls by a stable id, the two mounted
/// pickers never share one, and manual mode is reset with the draft it
/// belongs to.
#[test]
fn picker_links_help_and_resets_with_its_draft() {
    let picker = without_whitespace(&production(PICKER));
    assert!(picker.contains("aria-describedby=help_id.clone()"));
    assert!(picker.contains("aria-describedby=help_id_input.clone()"));
    assert!(picker.contains("id=help_id>"));
    assert!(picker
        .contains("Effect::new(move|_:Option<()>|{reset.track();manual_requested.set(false);});"));

    let room_list = without_whitespace(&production(ROOM_LIST));
    let binding = without_whitespace(&production(BINDING));
    assert!(room_list.contains("id_base=\"rooms-create-workspace\""));
    assert!(binding.contains("id_base=\"rooms-details-workspace\""));
    assert!(room_list.contains("reset=create_workspace_reset"));
    assert!(binding.contains("reset=picker_reset"));
    assert!(
        room_list.contains("create_workspace.set(String::new());create_workspace_reset.update("),
        "a successful create must clear the draft AND reset the picker's manual mode",
    );
    assert!(
        binding.contains("draft.set(current.1.clone().unwrap_or_default());picker_reset.update("),
        "reseeding the bind draft for another room or binding must reset the picker's manual mode",
    );
    assert!(
        binding.contains("ifreseed_needed(previous.as_ref(),current){"),
        "a roster frame that keeps id and root must not wipe a path mid-type",
    );
}

/// Keyboard order is DOM order: the select, then the revealed path field,
/// then the help text; Enter in the create form's path field submits the
/// create, and in the bind form it binds.
#[test]
fn picker_keyboard_order_and_enter_wiring() {
    let picker = production(PICKER);
    let view_at = picker.find("view! {").expect("picker view");
    let view = &picker[view_at..];
    let select_at = view.find("<select").expect("select");
    let input_at = view.find("<input").expect("manual input");
    let help_at = view
        .find("class=\"working-directory__help\"")
        .expect("help");
    assert!(select_at < input_at && input_at < help_at);
    assert!(
        without_whitespace(view).contains("ifev.key()==\"Enter\"{ifletSome(submit)=on_enter{ev.prevent_default();submit.run(());}}"),
        "Enter in the path field runs the owner's submit when one is given",
    );
    let room_list = without_whitespace(&production(ROOM_LIST));
    assert!(room_list.contains("on_enter=Callback::new(move|()|create_room())"));
    let binding = without_whitespace(&production(BINDING));
    assert!(binding.contains("on_enter=Callback::new(move|()|bind())"));
    // The Bind button is the bound-check's twin: never sends an unchanged or
    // blank root, never while a PATCH is in flight.
    assert!(binding.contains(
        "disabled=move||{!bind_enabled(&draft.get(),current_root.get().as_deref(),in_flight(),)}"
    ));
}
