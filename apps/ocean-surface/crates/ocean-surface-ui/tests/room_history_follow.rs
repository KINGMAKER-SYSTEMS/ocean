//! Rooms DoD 1.5 wiring: the open room's forward history arrives on exactly one
//! path, and live-follow never yanks a reader who scrolled up.
//!
//! The rules live in pure helpers with their own unit tests
//! (`rooms_workspace::read_cursor::transcript_pass_action`). What those tests
//! cannot see is the wiring in `rooms.rs` and `rooms_workspace/mod.rs`, so the
//! needles below run over production source only (the bottom `mod tests` is cut
//! off) with whitespace stripped so rustfmt may rewrap freely.
//!
//! Ported from Risingtides-dev/ocean-surface#218 (forward ingest left to SSE)
//! and #232 (follow-intent capture and DoD 1.5 follow guards).

use std::path::Path;

/// The production half of a `src/` module: everything above its trailing
/// `#[cfg(test)] mod tests`, so a needle the module's own unit tests could
/// quote does not satisfy a scan. Split on the module, not the first
/// `#[cfg(test)]`, because both files carry earlier item-level ones.
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

fn rooms() -> String {
    production("rooms.rs")
}

fn workspace() -> String {
    production("rooms_workspace/mod.rs")
}

/// Mutation responses must not start a second forward reader. The SSE tail
/// owns ordered replay and resumes from its own cursor after a reconnect; a
/// request-time `/transcript` read after join, leave, agent add or post races
/// that stream for the same rows.
///
/// The one permitted `/transcript` reader is the open catch-up: a bounded,
/// one-time fold over the pages behind the daemon's capped open response, run
/// before the open publishes its transcript and before the tail starts. It is
/// not polling and it is not a mutation re-read; see
/// `open_catch_up_is_the_only_transcript_reader_and_lands_before_the_tail`.
#[test]
fn mutation_responses_leave_forward_ingest_to_the_live_tail() {
    let rooms = rooms();
    assert!(
        !rooms.contains("refresh_open_transcript"),
        "mutation responses must not reintroduce the competing forward reader; \
         the room SSE tail is the sole forward-ingest path",
    );
    // The brace form is the URL as code builds it; doc comments name the
    // route without one.
    assert_eq!(
        rooms.matches("/transcript?after_seq={").count(),
        1,
        "exactly one production path reads `/transcript` forward (the open \
         catch-up); everything after open resumes from `/events?after_seq=`",
    );
    assert!(
        rooms.contains(
            "letevents_url=format!(\"{base}/v1/rooms/persistent/{}/events\",encode(&key));"
        ),
        "the room-scoped SSE tail stays the forward stream",
    );
}

/// The open catch-up (`OpenCatchUp` + `fetch_open_catch_up_page`) is reached
/// from `open_room` alone, and `open_room` lands the folded transcript before
/// it publishes rows or starts the tail, so the first fill is the newest,
/// contiguous transcript and the tail resumes from its real frontier.
#[test]
fn open_catch_up_is_the_only_transcript_reader_and_lands_before_the_tail() {
    let rooms = rooms();
    let reader = "{base}/v1/rooms/persistent/{}/transcript?after_seq={after_seq}&limit={OPEN_CATCH_UP_PAGE_LIMIT}";
    assert!(
        rooms.contains(reader),
        "the catch-up asks for the daemon's page ceiling from the held frontier",
    );
    let call = "fetch_open_catch_up_page(&base,&key,after_seq).await";
    assert_eq!(
        rooms.matches(call).count(),
        1,
        "the forward transcript page is fetched from exactly one call site",
    );
    let open_start = rooms
        .find("pubfnopen_room(&self,key:String){")
        .expect("open_room exists");
    let open_end = rooms[open_start..]
        .find("pubfnclose_room(&self){")
        .map(|offset| open_start + offset)
        .expect("close_room follows open_room");
    let open = &rooms[open_start..open_end];
    let call_at = open
        .find(call)
        .expect("the one call site is inside open_room");
    assert!(
        open.contains(
            "whileletSome(after_seq)=catch_up.next_after_seq(){letpage=fetch_open_catch_up_page(&base,&key,after_seq).await;if!me.room_is_current(generation_id,&key){return;}"
        ),
        "each page is followed by the room-identity check that cancels a \
         stale open on room switch",
    );
    let landed = open[call_at..]
        .find("lettranscript=catch_up.finish();")
        .expect("the fold lands after the last page");
    let published = open[call_at..]
        .find("me.transcript.set(transcript.clone());")
        .expect("the landed transcript is what gets published");
    let tail = open[call_at..]
        .find("me.start_live_tail(key,generation_id);")
        .expect("the tail starts after the open publishes");
    assert!(
        landed < published && published < tail,
        "land the whole transcript, publish it once, then tail from its frontier",
    );
}

/// A scrolled-up reader gets the affordance, never the scroll. The arm is
/// pinned WHOLE, because the failure is an addition to it, not a deletion.
#[test]
fn live_follow_raises_the_jump_and_never_moves_a_scrolled_reader() {
    let workspace = workspace();
    assert!(
        workspace.contains(
            "TranscriptPassAction::RaiseJump=>{new_below.set(true);pending_read_advance.set(None);}"
        ),
        "an append below a reader who scrolled up raises \"New messages\", \
         queues no read, and touches the scroll position NOT AT ALL",
    );
    assert!(
        workspace.contains("TranscriptPassAction::Hold=>{}"),
        "a pass that is neither an append nor a reset does nothing",
    );
}

/// Every programmatic scroll write in the workspace is one of two: the pin and
/// the jump button. A third is a new way to move a reader who did not ask to
/// move, and must be reconciled with DoD 1.5 before this count changes.
#[test]
fn the_transcript_scrolls_itself_in_exactly_two_places() {
    let workspace = workspace();
    assert_eq!(
        workspace.matches("set_scroll_top(").count(),
        2,
        "a new programmatic scroll write in `rooms_workspace/mod.rs` must be \
         reconciled with DoD 1.5 (\"live-follow never yanks a scrolled \
         reader\") and this count updated deliberately",
    );
    assert!(
        workspace.contains(
            "TranscriptPassAction::PinAndQueue=>{transcript_follow_pinned.set(true);let(scroll_height,_,client_height)=metrics.unwrap_or_default();ifletSome(el)=el.clone(){request_animation_frame(move||el.set_scroll_top(el.scroll_height()));}new_below.set(false);"
        ),
        "the pin: a first fill and a pinned reader follow the tail, and the \
         affordance goes away",
    );
}

/// Return-to-latest: the jump button and a manual scroll to the bottom both
/// restore follow intent, and the pass consumes that captured intent rather
/// than post-render geometry.
#[test]
fn return_to_latest_scrolls_to_the_newest_row_and_re_pins() {
    let workspace = workspace();
    assert!(
        workspace.contains(
            "on:click=move|_|{ifletSome(el)=list_ref.get(){el.set_scroll_top(el.scroll_height());}transcript_follow_pinned.set(true);new_below.set(false);queue_bottom_read_advance();}"
        ),
        "the \"New messages\" control scrolls to the newest row, re-pins, \
         clears itself, and marks what it brought on screen read",
    );
    assert!(
        workspace.contains(
            "letat_bottom=transcript_is_near_bottom(el.scroll_height(),el.scroll_top(),el.client_height(),120,);transcript_follow_pinned.set(at_bottom);ifat_bottom{new_below.set(false);queue_bottom_read_advance();}"
        ),
        "the scroll handler is where follow intent is captured, and scrolling \
         back down by hand clears the affordance",
    );
    assert!(
        workspace.contains(
            "letfollow_pinned=transcript_follow_pinned.get_untracked();matchtranscript_pass_action(len,prev_len,el.is_some(),follow_pinned)"
        ),
        "the pass that decides pin-or-jump must consume intent captured before \
         the append changed layout; post-render geometry alone misclassifies a \
         pinned reader after a tall or batched append",
    );
    assert!(
        workspace.contains("transcript_follow_pinned.set(true);new_below.set(false);pending_read_advance.set(None);}TranscriptPassAction::PinAndQueue=>"),
        "a room switch re-pins so the next room opens following its tail",
    );
}
