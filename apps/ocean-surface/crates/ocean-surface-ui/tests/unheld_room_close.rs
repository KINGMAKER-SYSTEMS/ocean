//! Rooms DoD 4.3 wiring: the close control is reachable, and it reaches the
//! daemon's close route through the member lane.
//!
//! The rules live in pure helpers with their own unit tests
//! (`room_close::{close_offered, classify_close, tab_trap}`). What those
//! tests cannot see is the wiring: whether the overflow item actually ARMS
//! the confirmation, whether the confirmation's button actually FIRES, and
//! whether the dialog is mounted where a header rebuild cannot drop it. The
//! shape that hides: delete the arming `on:click` and `state.confirm` is
//! still read by the render it gates and still reset by `reset`/`dismiss`,
//! so `cargo clippy -p ocean-surface-ui --target wasm32-unknown-unknown --
//! -D warnings` stays green while the daemon's `POST .../close` is
//! implemented, mounted and unreachable.
//!
//! Needles run over production source only (each module's bottom `mod tests`
//! is cut off) with whitespace stripped so rustfmt may rewrap freely.
//!
//! Ported from Risingtides-dev/ocean-surface#229 (`unheld_room_controls.rs`),
//! re-pointed at the monorepo's single header overflow (`room_overflow.rs`).

use std::path::Path;

/// The production half of a `src/` module: everything above its trailing
/// `#[cfg(test)] mod tests`, whitespace stripped.
fn production(rel: &str) -> String {
    let path = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/src")).join(rel);
    let source = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read src/{rel}: {e}"));
    let (production, _) = source
        .rsplit_once("#[cfg(test)]\nmod tests {")
        .unwrap_or_else(|| panic!("src/{rel} carries its unit tests at the bottom"));
    production
        .chars()
        .filter(|char| !char.is_whitespace())
        .collect()
}

/// The overflow's `Close room…` item is the only door to the confirmation,
/// and it only ARMS it — the dialog's own button fires. Deleting this click
/// alone leaves the wasm clippy green (`arm` is `pub(crate)` and `confirm`
/// is still read by the dialog's render), so it is pinned here.
#[test]
fn the_overflow_item_arms_the_close_confirmation() {
    let overflow = production("room_overflow.rs");
    assert!(
        overflow.contains("on:click=move|_|{open.set(false);close.arm();}"),
        "the overflow's `Close room…` item must arm the confirmation and nothing else; \
         without this click the dialog can never be reached",
    );
    assert!(
        overflow.contains("can_close().then(||view!{"),
        "the item must render only for a viewer `close_offered` admits — absence, \
         not a disabled button, for anyone the daemon's member lane would refuse",
    );
}

/// The confirmation's `Close room` button is the only caller of
/// `RoomCloseState::fire`, which alone keeps the request pipeline alive.
/// Deleting the click takes `fire`, `post_close`, `room_close_url` and
/// `classify_close` dead under `-D warnings` — compiler-held — but the pin
/// is cheap and names the contract: the fire goes through `dispatch_check`
/// and the member-lane URL, never an operator header.
#[test]
fn the_confirmation_button_fires_through_the_member_lane() {
    let close = production("room_close.rs");
    assert!(
        close.contains("on:click=move|_|state.fire(rooms)"),
        "the dialog's `Close room` button must be the one that fires",
    );
    assert!(
        close.contains("dispatch_check(actor,rooms.open_room.get_untracked().as_ref())"),
        "the fire path must re-check the member rule at dispatch time",
    );
    assert!(
        close.contains("/close?actor_id="),
        "the close goes through the daemon's member lane (`?actor_id=`)",
    );
    // The daemon picks the operator lane by the PRESENCE of a header, so the
    // one POST this module sends must set none: a bare builder, straight to
    // `send`. (The lane is named in doc comments, so the needle is the code
    // shape, not the header's name.)
    assert!(
        close.contains("Request::post(url).send().await"),
        "the close POST must carry no header: the operator lane is not roster-checked",
    );
    assert!(
        !close.contains(".header("),
        "no request in the close module may set a header",
    );
}

/// The state and the dialog live at `RoomsWorkspace` scope, outside the
/// header closure that is rebuilt on every `open_room` write. Constructed
/// inside it, an in-flight close would be re-enabled by a roster SSE frame
/// landing mid-request; mounted inside it, the dialog would unmount.
#[test]
fn the_close_state_and_dialog_outlive_the_header_closure() {
    let workspace = production("rooms_workspace/mod.rs");
    let state_at = workspace
        .find("letclose_control=crate::room_close::RoomCloseState::new(overflow_ref);")
        .expect("the close state is constructed once at workspace scope");
    let dialog_at = workspace
        .find("<crate::room_close::RoomCloseDialogrooms=roomsstate=close_control/>")
        .expect("the dialog is mounted once");
    let header_closure_at = workspace
        .find("letopen=rooms.open_room.get();matchopen{")
        .expect("the center rail's header closure");
    assert!(
        state_at < header_closure_at && dialog_at < header_closure_at,
        "state ({state_at}) and dialog ({dialog_at}) must precede the header closure \
         ({header_closure_at}), not live inside it",
    );
    assert!(
        workspace.contains("close=close_control"),
        "the overflow receives the shared state rather than making its own",
    );
}
