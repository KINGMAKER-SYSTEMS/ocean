//! Transcript scroll position and durable read-cursor advance decisions.

use crate::rooms::{RoomAccessProjection, RoomAccessState, RoomMessage, RoomReadCursorProjection};

pub(super) fn transcript_is_near_bottom(
    scroll_height: i32,
    scroll_top: i32,
    client_height: i32,
    threshold: i32,
) -> bool {
    scroll_height - scroll_top - client_height < threshold
}

/// Whether the transcript element can scroll at all. A transcript whose
/// content fits inside its viewport never fires a `scroll` event, so it can
/// never be *confirmed* at-bottom by scrolling — it simply already is.
pub(super) fn transcript_is_scrollable(scroll_height: i32, client_height: i32) -> bool {
    scroll_height > client_height
}

/// Whether this transcript pass is hydrated enough for its at-bottom state to
/// advance the durable read cursor.
///
/// Every pass after the first fill is hydrated. The first fill itself is
/// trusted only when the transcript is *measured* and cannot scroll: a
/// scrollable first fill is programmatically pinned to the bottom by the
/// transcript Effect, and the `scroll` event that pin produces is what
/// re-enters the read path with hydration complete (so a reader who
/// immediately scrolls up is never marked read from the raw fill). A
/// non-scrollable transcript never fires that event, so its first fill — by
/// definition entirely visible with the newest message at the bottom — must
/// advance the cursor directly, or the room stays unread forever.
///
/// "Measured" is the load-bearing word: an element that has not been laid out
/// yet reports `client_height == 0`, and `0 > 0` is false, so an unmeasured
/// first fill would otherwise masquerade as a fully visible transcript and
/// mark an arbitrarily long room read without a single visible message. Zero
/// (or negative) viewport height means "unknown", never "everything fits";
/// such a pass defers to the next one, which measures a laid-out element.
///
/// This is scroll-position hydration only. The room/access/transcript
/// hydration guards stay in [`ready_read_target`]: an open room, a non-empty
/// transcript, and a durable candidate from a `Local`/`Live` access
/// projection.
pub(super) fn transcript_read_hydrated(
    first_fill: bool,
    scroll_height: i32,
    client_height: i32,
) -> bool {
    if !first_fill {
        return true;
    }
    client_height > 0 && !transcript_is_scrollable(scroll_height, client_height)
}

pub(super) fn durable_read_candidate(
    transcript: &[RoomMessage],
    access: Option<&RoomAccessProjection>,
) -> Option<u64> {
    match access.map(|projection| projection.state) {
        Some(RoomAccessState::Live) => {
            access.and_then(|projection| projection.last_confirmed_global_sequence)
        }
        Some(RoomAccessState::Local) => transcript.last().map(|message| message.seq),
        _ => None,
    }
}

/// The single evaluation point for "may this state advance the durable read
/// cursor, and to which sequence?". [`read_advance_request`] derives from it,
/// so readiness and the candidate can never disagree and no caller needs an
/// unwrap/expect to recover the sequence.
///
/// These are the hydration guards proper: the room detail must be loaded, the
/// transcript non-empty, and the access projection must be `Local`/`Live`
/// enough to yield a durable candidate. `transcript_hydrated` carries only the
/// scroll-position question (see [`transcript_read_hydrated`]).
pub(super) fn ready_read_target(
    transcript_hydrated: bool,
    near_bottom: bool,
    room_loaded: bool,
    transcript: &[RoomMessage],
    access: Option<&RoomAccessProjection>,
) -> Option<u64> {
    if !transcript_hydrated || !near_bottom || !room_loaded || transcript.is_empty() {
        return None;
    }
    durable_read_candidate(transcript, access)
}

/// A queued read-advance intent, stamped with the room `generation` live at
/// the moment it was computed. `open_room_key` alone is not enough to prove
/// the request still belongs to the currently open room admission: closing
/// and reopening the *same* key bumps `Rooms::generation` without changing
/// the key, so a request built against the old admission must be rejected
/// even though `open_room_key` still matches. Re-validate with
/// `Rooms::room_is_current(generation, &open_room_key)` before dispatch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ReadAdvanceRequest {
    pub(super) open_room_key: String,
    pub(super) generation: u64,
    pub(super) candidate_read_seq: u64,
}

pub(super) fn read_advance_request(
    open_room_key: Option<&str>,
    generation: u64,
    transcript_hydrated: bool,
    near_bottom: bool,
    room_loaded: bool,
    transcript: &[RoomMessage],
    access: Option<&RoomAccessProjection>,
) -> Option<ReadAdvanceRequest> {
    let open_room_key = open_room_key?;
    let candidate_read_seq = ready_read_target(
        transcript_hydrated,
        near_bottom,
        room_loaded,
        transcript,
        access,
    )?;
    Some(ReadAdvanceRequest {
        open_room_key: open_room_key.to_string(),
        generation,
        candidate_read_seq,
    })
}

/// What one transcript pass should do. Extracting the decision keeps the
/// Effect body a dispatcher and makes the reactive re-run behaviour testable
/// without a browser: the same pass now fires for a transcript append *and*
/// for a room-access projection arriving after the fill.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TranscriptPassAction {
    /// No transcript (room switch / generation reset): drop the jump
    /// affordance and any queued read intent.
    Reset,
    /// First fill, or an at-bottom reader: pin to the newest message and
    /// (re)evaluate the durable read advance.
    PinAndQueue,
    /// Content appended below a scrolled-up reader: raise the jump
    /// affordance instead of yanking them.
    RaiseJump,
    /// Nothing to do. Critically, `Hold` never writes: a pass triggered by a
    /// non-transcript dependency (an access projection) while the reader is
    /// scrolled up must not mark read, must not raise the jump affordance,
    /// and must not clobber an already queued request.
    Hold,
}

/// `measured` is whether the transcript element exists *and* reports a real
/// viewport; an unmeasured pass holds so the fill state stays intact for the
/// first pass that can actually measure.
pub(super) fn transcript_pass_action(
    len: usize,
    prev_len: usize,
    measured: bool,
    near_bottom: bool,
) -> TranscriptPassAction {
    if len == 0 {
        return TranscriptPassAction::Reset;
    }
    if !measured {
        return TranscriptPassAction::Hold;
    }
    if prev_len == 0 || near_bottom {
        return TranscriptPassAction::PinAndQueue;
    }
    if len > prev_len {
        return TranscriptPassAction::RaiseJump;
    }
    TranscriptPassAction::Hold
}

/// The read position already durably applied to the open room: the furthest of
/// the room-list summary's confirmed read and the durable cursor projection
/// (own read plus mirrored upstream read). This mirrors the monotonic fold
/// `rooms.rs` applies before it will issue `PATCH /read-cursor`; here it is a
/// read-only pre-filter over public signals, so being conservative only costs
/// a redundant queue, never a lost read.
pub(super) fn applied_read_seq(
    summary_read_seq: Option<u64>,
    cursor: Option<&RoomReadCursorProjection>,
) -> Option<u64> {
    [
        summary_read_seq,
        cursor.and_then(|cursor| cursor.read_seq),
        cursor.and_then(|cursor| cursor.mirrored_upstream_read_seq),
    ]
    .into_iter()
    .flatten()
    .max()
}

/// Whether an at-bottom read target still needs to be queued.
///
/// A near-bottom scroll burst fires many events per second, each computing the
/// identical `(room, generation, seq)` target; allocating and publishing that
/// duplicate every frame churns the dispatch Effect for no state change. Two
/// self-releasing skips, both keyed on room/generation/seq:
///
/// - the same target is already queued and undispatched, or
/// - the durable cursor has already been confirmed at or past this sequence.
///
/// Neither can suppress a required retry. A failed `PATCH /read-cursor`
/// advances no cursor and the dispatch Effect has already cleared the pending
/// request, so the next near-bottom frame recomputes the same target and finds
/// both skips released.
pub(super) fn read_advance_needs_queue(
    pending: Option<&ReadAdvanceRequest>,
    open_room_key: &str,
    generation: u64,
    candidate_read_seq: u64,
    applied_read_seq: Option<u64>,
) -> bool {
    if applied_read_seq.is_some_and(|applied| applied >= candidate_read_seq) {
        return false;
    }
    !pending.is_some_and(|pending| {
        pending.open_room_key == open_room_key
            && pending.generation == generation
            && pending.candidate_read_seq == candidate_read_seq
    })
}

#[cfg(test)]
mod tests {
    use super::super::test_support::{test_access, test_msg};
    use super::*;
    use crate::rooms::room_request_is_current;

    #[test]
    fn read_advance_request_skips_hydration_but_advances_after_bottom_append() {
        let transcript = vec![test_msg(4, "hello", None), test_msg(7, "world", None)];
        let local = test_access(RoomAccessState::Local);

        assert_eq!(
            read_advance_request(None, 1, false, true, true, &transcript, Some(&local)),
            None
        );
        assert_eq!(
            read_advance_request(
                Some("room-a"),
                1,
                false,
                true,
                true,
                &transcript,
                Some(&local)
            ),
            None
        );
        assert_eq!(
            read_advance_request(
                Some("room-a"),
                1,
                true,
                true,
                true,
                &transcript,
                Some(&local)
            ),
            Some(ReadAdvanceRequest {
                open_room_key: "room-a".into(),
                generation: 1,
                candidate_read_seq: 7,
            })
        );
    }

    /// F1 regression: the *first fill* of a room whose transcript fits inside
    /// the viewport must advance the durable read cursor. A scrollable first
    /// fill still defers to the `scroll` event the programmatic bottom-pin
    /// produces, so a reader who scrolls up is never marked read from the raw
    /// fill.
    #[test]
    fn transcript_read_hydration_trusts_non_scrollable_first_fill_only() {
        // Content fits the viewport: nothing can scroll, nothing will ever
        // confirm the position — the fill itself is the confirmation.
        assert!(!transcript_is_scrollable(400, 400));
        assert!(!transcript_is_scrollable(120, 400));
        assert!(transcript_read_hydrated(true, 400, 400));
        assert!(transcript_read_hydrated(true, 120, 400));

        // Scrollable first fill: defer to the scroll event.
        assert!(transcript_is_scrollable(4000, 400));
        assert!(!transcript_read_hydrated(true, 4000, 400));

        // Every later pass is hydrated regardless of scrollability.
        assert!(transcript_read_hydrated(false, 4000, 400));
        assert!(transcript_read_hydrated(false, 400, 400));
    }

    /// M1 regression: an unmeasured transcript (`client_height == 0`, the
    /// reading a not-yet-laid-out element gives) must NOT be mistaken for a
    /// transcript that fits its viewport. `scroll_height > client_height` is
    /// false for `0 > 0`, so without the positive-height requirement the very
    /// first fill of an arbitrarily long room would mark the whole room read
    /// before a single message was visible.
    #[test]
    fn transcript_read_hydration_rejects_unmeasured_first_fill() {
        assert!(!transcript_read_hydrated(true, 0, 0));
        // Content already measured but the viewport is not: still unknown.
        assert!(!transcript_read_hydrated(true, 4000, 0));
        // Nonsense/negative heights are unknown too, never "everything fits".
        assert!(!transcript_read_hydrated(true, 0, -1));
        // A measured viewport keeps the F1 behaviour exactly.
        assert!(transcript_read_hydrated(true, 240, 600));
        assert!(!transcript_read_hydrated(true, 4000, 600));
        // Later passes are unaffected: hydration is a first-fill question.
        assert!(transcript_read_hydrated(false, 0, 0));
    }

    /// M1 regression through the production decision path: the exact
    /// arguments the transcript Effect passes for an unmeasured first fill
    /// produce no request, even though the unmeasured element reports a
    /// trivially "near bottom" position (`0 - 0 - 0 < 120`).
    #[test]
    fn read_advance_request_skips_unmeasured_first_fill() {
        let transcript = vec![test_msg(4, "hello", None), test_msg(7, "world", None)];
        let local = test_access(RoomAccessState::Local);
        assert!(transcript_is_near_bottom(0, 0, 0, 120));

        assert_eq!(
            read_advance_request(
                Some("room-a"),
                2,
                transcript_read_hydrated(true, 0, 0),
                transcript_is_near_bottom(0, 0, 0, 120),
                true,
                &transcript,
                Some(&local),
            ),
            None
        );
    }

    /// L2 regression: the transcript pass is now driven by the access
    /// projection as well as by transcript length, so a pass can re-run with
    /// an unchanged transcript. Such a pass must requeue when the reader is at
    /// the bottom and must do nothing at all when they are scrolled up — no
    /// read mark, no jump affordance, no clobbering of a queued request.
    #[test]
    fn transcript_pass_action_covers_append_reset_and_unchanged_reruns() {
        // Room switch / generation reset.
        assert_eq!(
            transcript_pass_action(0, 5, true, true),
            TranscriptPassAction::Reset
        );
        // First fill, regardless of the measured scroll position.
        assert_eq!(
            transcript_pass_action(5, 0, true, false),
            TranscriptPassAction::PinAndQueue
        );
        // Access arrives after the fill, reader still at the bottom: requeue.
        assert_eq!(
            transcript_pass_action(5, 5, true, true),
            TranscriptPassAction::PinAndQueue
        );
        // Access arrives after the fill, reader scrolled up: hold.
        assert_eq!(
            transcript_pass_action(5, 5, true, false),
            TranscriptPassAction::Hold
        );
        // Append below a scrolled-up reader keeps the jump affordance.
        assert_eq!(
            transcript_pass_action(6, 5, true, false),
            TranscriptPassAction::RaiseJump
        );
        // Append while at the bottom pins and queues.
        assert_eq!(
            transcript_pass_action(6, 5, true, true),
            TranscriptPassAction::PinAndQueue
        );
        // No transcript element yet: hold, so the first-fill state survives
        // for the pass that can actually measure.
        assert_eq!(
            transcript_pass_action(5, 0, false, true),
            TranscriptPassAction::Hold
        );
    }

    /// L2 regression, end to end through the production decision path: a
    /// `Live` room fills before the daemon confirms a global sequence, so the
    /// fill itself has no durable candidate. When the access projection lands
    /// afterwards the pass re-runs with an unchanged transcript; at the bottom
    /// that pass must produce exactly the request the fill could not.
    #[test]
    fn live_room_requeues_read_when_confirmed_sequence_lands_after_fill() {
        let transcript = vec![test_msg(4, "hello", None), test_msg(7, "world", None)];
        let (scroll_height, client_height) = (240, 600);
        let near_bottom = transcript_is_near_bottom(scroll_height, 0, client_height, 120);

        // Fill: `Live` access with no confirmed sequence yet.
        let mut live = test_access(RoomAccessState::Live);
        assert_eq!(
            transcript_pass_action(transcript.len(), 0, true, near_bottom),
            TranscriptPassAction::PinAndQueue
        );
        assert_eq!(
            read_advance_request(
                Some("room-a"),
                3,
                transcript_read_hydrated(true, scroll_height, client_height),
                near_bottom,
                true,
                &transcript,
                Some(&live),
            ),
            None
        );

        // The confirmed global sequence arrives; the transcript did not change.
        live.last_confirmed_global_sequence = Some(44);
        assert_eq!(
            transcript_pass_action(transcript.len(), transcript.len(), true, near_bottom),
            TranscriptPassAction::PinAndQueue
        );
        assert_eq!(
            read_advance_request(
                Some("room-a"),
                3,
                // Not a first fill any more, so hydration is settled.
                transcript_read_hydrated(false, scroll_height, client_height),
                near_bottom,
                true,
                &transcript,
                Some(&live),
            ),
            Some(ReadAdvanceRequest {
                open_room_key: "room-a".into(),
                generation: 3,
                candidate_read_seq: 44,
            })
        );

        // Same arrival while the reader is scrolled up marks nothing.
        assert_eq!(
            transcript_pass_action(transcript.len(), transcript.len(), true, false),
            TranscriptPassAction::Hold
        );
    }

    /// L3: the applied read position is the monotonic fold of the room-list
    /// summary and both halves of the durable cursor projection, so a lagging
    /// half can never mask a further confirmed read.
    #[test]
    fn applied_read_seq_folds_summary_and_cursor_monotonically() {
        assert_eq!(applied_read_seq(None, None), None);
        assert_eq!(applied_read_seq(Some(5), None), Some(5));
        assert_eq!(
            applied_read_seq(
                None,
                Some(&RoomReadCursorProjection {
                    read_seq: Some(9),
                    mirrored_upstream_read_seq: None,
                })
            ),
            Some(9)
        );
        assert_eq!(
            applied_read_seq(
                Some(3),
                Some(&RoomReadCursorProjection {
                    read_seq: Some(9),
                    mirrored_upstream_read_seq: Some(12),
                })
            ),
            Some(12)
        );
    }

    /// L3 regression: a near-bottom scroll burst recomputes the identical
    /// target every frame. The duplicate is skipped while it is still queued
    /// and once the daemon has confirmed it, but a failed PATCH — which clears
    /// the pending request and advances no cursor — must still be retried.
    #[test]
    fn read_advance_needs_queue_dedupes_without_suppressing_retry() {
        let pending = ReadAdvanceRequest {
            open_room_key: "room-a".into(),
            generation: 2,
            candidate_read_seq: 7,
        };

        // Identical target already queued: skip.
        assert!(!read_advance_needs_queue(
            Some(&pending),
            "room-a",
            2,
            7,
            None
        ));
        // Same key/seq under a new admission is a different target.
        assert!(read_advance_needs_queue(
            Some(&pending),
            "room-a",
            3,
            7,
            None
        ));
        // Different room, and a further sequence in the same room, both queue.
        assert!(read_advance_needs_queue(
            Some(&pending),
            "room-b",
            2,
            7,
            None
        ));
        assert!(read_advance_needs_queue(
            Some(&pending),
            "room-a",
            2,
            8,
            None
        ));
        // Already durably read at or past the candidate: nothing to advance.
        assert!(!read_advance_needs_queue(None, "room-a", 2, 7, Some(7)));
        assert!(!read_advance_needs_queue(None, "room-a", 2, 7, Some(9)));
        // Retry after failure: the dispatch Effect cleared the pending request
        // and the failed PATCH advanced no cursor, so the next near-bottom
        // frame must re-queue the very same target.
        assert!(read_advance_needs_queue(None, "room-a", 2, 7, None));
        assert!(read_advance_needs_queue(None, "room-a", 2, 7, Some(6)));
    }

    /// F1 regression, end to end through the production decision path: the
    /// exact arguments the transcript Effect passes for a non-scrollable
    /// first fill produce a read-advance request, while the scrollable first
    /// fill of the same room does not.
    #[test]
    fn read_advance_request_marks_non_scrollable_first_fill_at_bottom() {
        let transcript = vec![test_msg(4, "hello", None), test_msg(7, "world", None)];
        let local = test_access(RoomAccessState::Local);
        // Short transcript in a tall viewport: scroll_top is pinned at 0 and
        // `near_bottom` is trivially true.
        let (scroll_height, client_height) = (240, 600);
        let near_bottom = transcript_is_near_bottom(scroll_height, 0, client_height, 120);
        assert!(near_bottom);

        assert_eq!(
            read_advance_request(
                Some("room-a"),
                2,
                transcript_read_hydrated(true, scroll_height, client_height),
                near_bottom,
                true,
                &transcript,
                Some(&local),
            ),
            Some(ReadAdvanceRequest {
                open_room_key: "room-a".into(),
                generation: 2,
                candidate_read_seq: 7,
            })
        );

        // Same room, taller content than viewport: the first fill stays
        // silent and the pin's `scroll` event does the marking.
        assert_eq!(
            read_advance_request(
                Some("room-a"),
                2,
                transcript_read_hydrated(true, 4000, client_height),
                true,
                true,
                &transcript,
                Some(&local),
            ),
            None
        );
    }

    /// The non-scrollable shortcut must not bypass the room/access/transcript
    /// hydration guards, and must not fire while the reader is scrolled up.
    #[test]
    fn read_advance_request_non_scrollable_still_requires_hydration_and_bottom() {
        let transcript = vec![test_msg(7, "world", None)];
        let local = test_access(RoomAccessState::Local);
        let hydrated = transcript_read_hydrated(true, 240, 600);
        assert!(hydrated);

        // Open room not yet hydrated.
        assert_eq!(
            read_advance_request(
                Some("room-a"),
                2,
                hydrated,
                true,
                false,
                &transcript,
                Some(&local)
            ),
            None
        );
        // Access projection not yet hydrated.
        assert_eq!(
            read_advance_request(Some("room-a"), 2, hydrated, true, true, &transcript, None),
            None
        );
        // Transcript not yet hydrated.
        assert_eq!(
            read_advance_request(Some("room-a"), 2, hydrated, true, true, &[], Some(&local)),
            None
        );
        // Scrolled up: intent wins over the shortcut.
        assert_eq!(
            read_advance_request(
                Some("room-a"),
                2,
                hydrated,
                false,
                true,
                &transcript,
                Some(&local)
            ),
            None
        );
    }

    /// A `Live` room with no confirmed global sequence yields no candidate, so
    /// the request path returns `None` instead of panicking on a missing
    /// candidate (F5: one evaluation, no `expect`).
    #[test]
    fn read_advance_request_returns_none_when_live_has_no_confirmed_sequence() {
        let transcript = vec![test_msg(7, "world", None)];
        let live = test_access(RoomAccessState::Live);
        assert_eq!(live.last_confirmed_global_sequence, None);

        assert_eq!(
            ready_read_target(true, true, true, &transcript, Some(&live)),
            None
        );
        assert_eq!(
            read_advance_request(
                Some("room-a"),
                2,
                true,
                true,
                true,
                &transcript,
                Some(&live)
            ),
            None
        );
    }

    #[test]
    fn read_advance_request_allows_jump_to_latest_and_captures_generation() {
        let transcript = vec![test_msg(11, "hello", None)];
        let local = test_access(RoomAccessState::Local);

        assert_eq!(
            read_advance_request(
                Some("room-z"),
                5,
                true,
                true,
                true,
                &transcript,
                Some(&local)
            ),
            Some(ReadAdvanceRequest {
                open_room_key: "room-z".into(),
                generation: 5,
                candidate_read_seq: 11,
            })
        );
    }

    #[test]
    fn read_advance_request_rejects_scrolled_up_or_missing_candidate() {
        let transcript = vec![test_msg(5, "hello", None)];
        let connecting = test_access(RoomAccessState::Connecting);

        assert_eq!(
            read_advance_request(
                Some("room-a"),
                1,
                true,
                false,
                true,
                &transcript,
                Some(&test_access(RoomAccessState::Local))
            ),
            None
        );
        assert_eq!(
            read_advance_request(
                Some("room-a"),
                1,
                true,
                true,
                true,
                &transcript,
                Some(&connecting)
            ),
            None
        );
    }

    /// Exact regression for the same-key close/reopen gap: a request is
    /// scheduled (stamped `generation`) while room "A" is open at gen N, the
    /// SAME key is closed and reopened (gen N -> N+1, key unchanged), and the
    /// stale request must be rejected by the exact predicate the dispatch
    /// effect calls before `mark_open_read_if_current` —
    /// `crate::rooms::room_request_is_current`, the same logic
    /// `Rooms::room_is_current` delegates to (no live `Rooms`/browser runtime
    /// needed to exercise it here).
    #[test]
    fn read_advance_request_generation_is_rejected_after_same_key_close_reopen() {
        let transcript = vec![test_msg(9, "hello", None)];
        let local = test_access(RoomAccessState::Local);

        // Schedule A: room "A" is open at gen N; build a read-advance request
        // stamped with that live generation.
        let scheduled_generation = 3; // gen N
        let request = read_advance_request(
            Some("room-a"),
            scheduled_generation,
            true,
            true,
            true,
            &transcript,
            Some(&local),
        )
        .expect("ready state produces a request");
        assert_eq!(request.generation, scheduled_generation);
        assert_eq!(request.open_room_key, "room-a");

        // Close/reopen the SAME key: generation advances to N+1, `open_key`
        // is still "room-a" — the pre-fix key-only guard would wrongly admit
        // this stale request.
        let generation_after_close_reopen = scheduled_generation + 1;

        // The exact predicate the dispatch Effect calls before
        // `mark_open_read_if_current` must reject the stale request.
        assert!(!room_request_is_current(
            request.generation,
            generation_after_close_reopen,
            &request.open_room_key,
            Some("room-a"),
        ));
        // A freshly-stamped request for the new admission is admitted.
        assert!(room_request_is_current(
            generation_after_close_reopen,
            generation_after_close_reopen,
            "room-a",
            Some("room-a"),
        ));
    }

    #[test]
    fn transcript_bottom_threshold_matches_follow_contract() {
        assert!(transcript_is_near_bottom(1000, 810, 100, 120));
        assert!(!transcript_is_near_bottom(1000, 700, 100, 120));
    }

    #[test]
    fn durable_read_candidate_uses_local_transcript_and_live_global_only() {
        let transcript = vec![test_msg(4, "hello", None), test_msg(7, "world", None)];
        let local = test_access(RoomAccessState::Local);
        let mut live = test_access(RoomAccessState::Live);
        live.last_confirmed_global_sequence = Some(44);
        let connecting = test_access(RoomAccessState::Connecting);

        assert_eq!(durable_read_candidate(&transcript, Some(&local)), Some(7));
        assert_eq!(durable_read_candidate(&transcript, Some(&live)), Some(44));
        assert_eq!(durable_read_candidate(&transcript, Some(&connecting)), None);
        assert_eq!(durable_read_candidate(&[], Some(&local)), None);
    }

    #[test]
    fn ready_read_target_requires_hydrated_room_near_bottom_and_candidate() {
        let transcript = vec![test_msg(4, "hello", None)];
        let local = test_access(RoomAccessState::Local);

        assert_eq!(
            ready_read_target(true, true, true, &transcript, Some(&local)),
            Some(4)
        );
        assert_eq!(
            ready_read_target(false, true, true, &transcript, Some(&local)),
            None
        );
        assert_eq!(
            ready_read_target(true, false, true, &transcript, Some(&local)),
            None
        );
        assert_eq!(
            ready_read_target(true, true, false, &transcript, Some(&local)),
            None
        );
        assert_eq!(ready_read_target(true, true, true, &[], Some(&local)), None);
        assert_eq!(
            ready_read_target(
                true,
                true,
                true,
                &transcript,
                Some(&test_access(RoomAccessState::Connecting))
            ),
            None
        );
    }
}
