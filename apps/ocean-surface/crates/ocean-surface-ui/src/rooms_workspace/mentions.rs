//! Mention autosuggest: caret/token math, roster ranking, and acceptance.

use leptos::prelude::*;
use wasm_bindgen::JsCast;

use super::format::{agent_descriptor_line, avatar_identity_class, participant_kind_label};
use crate::rooms::{
    FederatedActorType, RoomAccessProjection, RoomAccessState, RoomParticipant,
    RoomParticipantKind, Rooms,
};

// ── Mention autosuggest (pure, unit-testable) ──────────────────────────────

/// A ranked mention suggestion for the composer typeahead popup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct MentionSuggestion {
    pub(super) id: String,
    pub(super) display_name: String,
    pub(super) kind: RoomParticipantKind,
}

/// Convert a UTF-16 code-unit offset (what `selectionStart` reports) into a
/// byte offset into `s`, clamped to the string end.
pub(super) fn utf16_to_byte_idx(s: &str, utf16: usize) -> usize {
    let mut units = 0usize;
    for (byte_idx, ch) in s.char_indices() {
        if units >= utf16 {
            return byte_idx;
        }
        units += ch.len_utf16();
    }
    s.len()
}

/// Convert a byte offset into `s` into a UTF-16 code-unit offset, clamped.
pub(super) fn byte_to_utf16_idx(s: &str, byte: usize) -> usize {
    s[..byte.min(s.len())].chars().map(|c| c.len_utf16()).sum()
}

/// If the caret sits directly after an `@token`, return the byte index of the
/// `@` plus the partial token typed so far. Mirrors the tokenizer's rules:
/// the `@` must start the text or follow a non-mention character (so email
/// local parts never trigger the popup), and every character between `@` and
/// the caret must be a mention character.
pub(super) fn mention_query(text: &str, cursor: usize) -> Option<(usize, String)> {
    if cursor > text.len() || !text.is_char_boundary(cursor) {
        return None;
    }
    let before = &text[..cursor];
    let at = before.rfind('@')?;
    let partial = &before[at + '@'.len_utf8()..];
    if !partial.chars().all(crate::room_markdown::is_mention_char) {
        return None;
    }
    if !crate::room_markdown::mention_start_boundary(before[..at].chars().next_back()) {
        return None;
    }
    Some((at, partial.to_string()))
}

pub(super) fn live_mention_query_from_input(
    text: &str,
    selection_start_utf16: Option<u32>,
) -> Option<(usize, String)> {
    let cursor = selection_start_utf16
        .map(|u| utf16_to_byte_idx(text, u as usize))
        .unwrap_or(text.len());
    mention_query(text, cursor)
}

/// Select the daemon-authoritative roster for mention completion. Local rooms
/// use `Room.participants`; every non-Local room uses only the safe access
/// member projection so stale/local identities never become mention ids.
pub(super) fn mention_roster(
    local_participants: &[RoomParticipant],
    access: Option<&RoomAccessProjection>,
) -> Vec<RoomParticipant> {
    match access {
        Some(access) if access.state == RoomAccessState::Local => local_participants.to_vec(),
        Some(access) => access
            .members
            .iter()
            .map(|member| RoomParticipant {
                id: member.member_id.clone(),
                kind: match member.actor_type {
                    FederatedActorType::User => RoomParticipantKind::Human,
                    FederatedActorType::Agent => RoomParticipantKind::Agent,
                },
                display_name: member.display_name.clone(),
            })
            .collect(),
        None => Vec::new(),
    }
}

/// Rank roster candidates for a mention partial: id prefix first, then
/// display-name prefix, then substring anywhere; stable within each rank and
/// capped at 8. An empty partial (caret right after `@`) lists the roster.
pub(super) fn mention_suggestions(
    participants: &[RoomParticipant],
    partial: &str,
) -> Vec<MentionSuggestion> {
    let q = partial.to_lowercase();
    let mut ranked: Vec<(u8, MentionSuggestion)> = participants
        .iter()
        .filter_map(|p| {
            let id = p.id.to_lowercase();
            let name = p.display_name.to_lowercase();
            let rank = if q.is_empty() || id.starts_with(&q) {
                0
            } else if name.starts_with(&q) {
                1
            } else if id.contains(&q) || name.contains(&q) {
                2
            } else {
                return None;
            };
            Some((
                rank,
                MentionSuggestion {
                    id: p.id.clone(),
                    display_name: p.display_name.clone(),
                    kind: p.kind,
                },
            ))
        })
        .collect();
    ranked.sort_by_key(|(rank, _)| *rank);
    ranked.into_iter().map(|(_, s)| s).take(8).collect()
}

pub(super) fn mention_suggestion_at(
    participants: &[RoomParticipant],
    partial: &str,
    index: usize,
) -> Option<MentionSuggestion> {
    mention_suggestions(participants, partial)
        .get(index)
        .cloned()
}

pub(super) fn mention_accept_is_valid(
    text: &str,
    selection_start_utf16: Option<u32>,
    participants: &[RoomParticipant],
    index: usize,
    displayed_id: Option<&str>,
) -> bool {
    live_mention_query_from_input(text, selection_start_utf16)
        .and_then(|(_, partial)| mention_suggestion_at(participants, &partial, index))
        .is_some_and(|candidate| displayed_id == Some(candidate.id.as_str()))
}

/// Replace the active mention token beginning at `at` with `@id `, extending
/// beyond the caret through any remaining mention characters, and return the
/// new text plus the byte caret position after one separator.
pub(super) fn apply_mention(text: &str, at: usize, cursor: usize, id: &str) -> (String, usize) {
    let cursor = cursor.min(text.len());
    let trailing_token_len = text[cursor..]
        .chars()
        .take_while(|&c| crate::room_markdown::is_mention_char(c))
        .map(char::len_utf8)
        .sum::<usize>();
    let replace_end = cursor + trailing_token_len;
    let suffix = &text[replace_end..];
    let suffix = match suffix.chars().next() {
        Some(c) if c.is_whitespace() => &suffix[c.len_utf8()..],
        _ => suffix,
    };
    let mut out = String::with_capacity(text.len() + id.len() + 2);
    out.push_str(&text[..at]);
    out.push('@');
    out.push_str(id);
    out.push(' ');
    let caret = out.len();
    out.push_str(suffix);
    (out, caret)
}

/// Keyboard model for the mention popup while it is open.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum MentionKey {
    Move(usize),
    Accept,
    Close,
    Pass,
}

pub(super) fn mention_popup_key(len: usize, active: usize, key: &str) -> MentionKey {
    if len == 0 {
        return MentionKey::Pass;
    }
    match key {
        "ArrowDown" => MentionKey::Move((active + 1) % len),
        "ArrowUp" => MentionKey::Move((active + len - 1) % len),
        "Enter" | "Tab" => MentionKey::Accept,
        "Escape" => MentionKey::Close,
        _ => MentionKey::Pass,
    }
}

// ── Mention controller (one per composer) ─────────────────────────────────

/// Autosuggest state for one composer input. The channel and thread composers
/// each own one; the state lives in `RoomsWorkspace` so it survives the
/// center/right rails re-rendering on room updates.
#[derive(Clone, Copy)]
pub(super) struct MentionState {
    pub(super) input_ref: NodeRef<leptos::html::Input>,
    pub(super) ctx: RwSignal<Option<(usize, String)>>,
    pub(super) active: RwSignal<usize>,
    pub(super) items: Memo<Vec<MentionSuggestion>>,
}

impl MentionState {
    pub(super) fn new(rooms: Rooms) -> Self {
        let input_ref: NodeRef<leptos::html::Input> = NodeRef::new();
        let ctx = RwSignal::new(None::<(usize, String)>);
        let active = RwSignal::new(0usize);
        let items = Memo::new(move |_| {
            let Some((_, partial)) = ctx.get() else {
                return Vec::new();
            };
            let local_participants = rooms
                .open_room
                .get()
                .map(|room| room.participants)
                .unwrap_or_default();
            let roster = mention_roster(&local_participants, rooms.access.get().as_ref());
            mention_suggestions(&roster, &partial)
        });
        Self {
            input_ref,
            ctx,
            active,
            items,
        }
    }

    fn close(self) {
        self.ctx.set(None);
        self.active.set(0);
    }

    /// Accept suggestion `idx` into `draft`, but only while the live caret
    /// still resolves to the candidate the popup displayed.
    pub(super) fn accept(
        self,
        rooms: Rooms,
        draft: RwSignal<String>,
        idx: usize,
        displayed_id: Option<String>,
    ) {
        let text = draft.get_untracked();
        let live_ctx = match self.input_ref.get() {
            Some(input) => match input.selection_start().ok().flatten() {
                Some(cursor) => live_mention_query_from_input(&text, Some(cursor)),
                None => self.ctx.get_untracked(),
            },
            None => self.ctx.get_untracked(),
        };
        let Some((at, partial)) = live_ctx else {
            self.close();
            return;
        };
        let local_participants = rooms
            .open_room
            .get_untracked()
            .map(|room| room.participants)
            .unwrap_or_default();
        let roster = mention_roster(&local_participants, rooms.access.get_untracked().as_ref());
        let Some(pick) = mention_suggestion_at(&roster, &partial, idx) else {
            self.close();
            return;
        };
        if displayed_id.as_deref() != Some(pick.id.as_str()) {
            self.close();
            return;
        }
        let cursor = at + 1 + partial.len();
        let (new_text, caret) = apply_mention(&text, at, cursor, &pick.id);
        let caret16 = byte_to_utf16_idx(&new_text, caret) as u32;
        draft.set(new_text);
        self.close();
        if let Some(input) = self.input_ref.get() {
            let _ = input.focus();
            let _ = input.set_selection_range(caret16, caret16);
        }
    }

    pub(super) fn on_input(self, draft: RwSignal<String>, ev: &web_sys::Event) {
        let value = event_target_value(ev);
        self.ctx.set(live_mention_query_from_input(
            &value,
            ev.target()
                .and_then(|t| t.dyn_into::<web_sys::HtmlInputElement>().ok())
                .and_then(|el| el.selection_start().ok().flatten()),
        ));
        self.active.set(0);
        draft.set(value);
    }

    pub(super) fn on_keydown(
        self,
        rooms: Rooms,
        draft: RwSignal<String>,
        ev: &web_sys::KeyboardEvent,
    ) {
        if ev.is_composing() {
            return;
        }
        if self.ctx.get_untracked().is_none() {
            return;
        }
        let key = ev.key();
        let active = self.active.get_untracked();
        if matches!(key.as_str(), "Enter" | "Tab") {
            let local_participants = rooms
                .open_room
                .get_untracked()
                .map(|room| room.participants)
                .unwrap_or_default();
            let roster = mention_roster(&local_participants, rooms.access.get_untracked().as_ref());
            let selection = ev
                .target()
                .and_then(|target| target.dyn_into::<web_sys::HtmlInputElement>().ok())
                .and_then(|input| input.selection_start().ok().flatten());
            let displayed_id = self
                .items
                .get_untracked()
                .get(active)
                .map(|item| item.id.clone());
            if !mention_accept_is_valid(
                &draft.get_untracked(),
                selection,
                &roster,
                active,
                displayed_id.as_deref(),
            ) {
                self.close();
                return;
            }
        }
        let len = self.items.get_untracked().len();
        match mention_popup_key(len, active, &key) {
            MentionKey::Move(next) => {
                ev.prevent_default();
                self.active.set(next);
            }
            MentionKey::Accept => {
                ev.prevent_default();
                let displayed_id = self
                    .items
                    .get_untracked()
                    .get(active)
                    .map(|item| item.id.clone());
                self.accept(rooms, draft, active, displayed_id);
            }
            MentionKey::Close => {
                ev.prevent_default();
                self.ctx.set(None);
            }
            MentionKey::Pass => {}
        }
    }

    /// `aria-activedescendant` for the combobox input: the active option's
    /// DOM id, or empty while the popup is closed.
    pub(super) fn active_descendant(self, option_id_prefix: &'static str) -> String {
        if self.items.get().is_empty() {
            String::new()
        } else {
            format!("{option_id_prefix}{}", self.active.get())
        }
    }

    /// The suggestion listbox, rendered only while there are suggestions.
    pub(super) fn popup(
        self,
        rooms: Rooms,
        draft: RwSignal<String>,
        listbox_id: &'static str,
        option_id_prefix: &'static str,
    ) -> impl IntoView {
        move || {
            let items = self.items.get();
            if items.is_empty() {
                return ().into_any();
            }
            let active = self.active.get();
            let access = rooms.access.get();
            view! {
                <div
                    id=listbox_id
                    class="rooms-workspace__mention-pop"
                    role="listbox"
                    aria-label="Mention suggestions"
                >
                    {items
                        .into_iter()
                        .enumerate()
                        .map(|(i, item)| {
                            let desc = agent_descriptor_line(access.as_ref(), &item.id);
                            let initials = crate::rooms::name_initials(&item.display_name);
                            let avatar_class = format!(
                                "rooms-workspace__member-avatar {}",
                                avatar_identity_class(&item.id)
                            );
                            let id_label = format!("@{}", item.id);
                            let clicked_id = item.id.clone();
                            view! {
                                <div
                                    id=format!("{option_id_prefix}{i}")
                                    class="rooms-workspace__mention-opt"
                                    class:rooms-workspace__mention-opt--active=i == active
                                    role="option"
                                    aria-selected=(i == active).to_string()
                                    on:mousedown=move |ev: web_sys::MouseEvent| {
                                        // Keep combobox focus until the click activation runs.
                                        ev.prevent_default();
                                    }
                                    on:click=move |_| {
                                        self.accept(rooms, draft, i, Some(clicked_id.clone()))
                                    }
                                >
                                    <span class=avatar_class>{initials}</span>
                                    <span class="rooms-workspace__mention-name">
                                        {item.display_name.clone()}
                                    </span>
                                    <span class="rooms-workspace__mention-id">{id_label}</span>
                                    <span class="rooms-workspace__mention-kind">
                                        {participant_kind_label(item.kind)}
                                    </span>
                                    {desc.map(|d| view! {
                                        <span class="rooms-workspace__mention-desc">{d}</span>
                                    })}
                                </div>
                            }
                        })
                        .collect_view()}
                </div>
            }
            .into_any()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_support::test_access;
    use super::*;
    use crate::rooms::{FederatedRoomMemberProjection, FederatedRoomRole};

    fn part(id: &str, name: &str, kind: RoomParticipantKind) -> RoomParticipant {
        RoomParticipant {
            id: id.into(),
            kind,
            display_name: name.into(),
        }
    }

    #[test]
    fn mention_query_detects_partial_at_caret() {
        assert_eq!(mention_query("hi @fl", 6), Some((3, "fl".to_string())));
        assert_eq!(mention_query("@", 1), Some((0, String::new())));
        assert_eq!(
            mention_query("say @designer x", 13),
            Some((4, "designer".to_string()))
        );
    }

    #[test]
    fn mention_query_rejects_email_local_parts_and_non_tokens() {
        // '@' directly after a mention char = email shape, never a popup.
        assert_eq!(mention_query("mail me a@b", 11), None);
        // Whitespace inside the candidate token closes the query.
        assert_eq!(mention_query("@fl x", 5), None);
        // No '@' at all.
        assert_eq!(mention_query("hello", 5), None);
    }

    #[test]
    fn mention_query_is_unicode_safe() {
        let text = "héllo @fl";
        assert_eq!(mention_query(text, text.len()), Some((7, "fl".to_string())));
        // Non-boundary cursor never panics.
        assert_eq!(mention_query("é@a", 1), None);
    }

    #[test]
    fn mention_query_rejects_unicode_letter_before_at() {
        assert_eq!(mention_query("é@fl", "é@fl".len()), None);
    }

    #[test]
    fn live_mention_query_uses_current_selection_start() {
        let text = "hello @fl tail";
        let stale_cursor = text.len();
        assert_eq!(mention_query(text, stale_cursor), None);
        assert_eq!(
            live_mention_query_from_input(text, Some(9)),
            Some((6, "fl".to_string()))
        );
        assert_eq!(live_mention_query_from_input(text, Some(5)), None);
    }

    #[test]
    fn mention_roster_uses_room_participants_only_for_local_access() {
        let local = vec![part("local-human", "Local", RoomParticipantKind::Human)];
        let mut access = test_access(RoomAccessState::Local);
        access.members = vec![FederatedRoomMemberProjection {
            member_id: "projected-agent".into(),
            owner_member_id: None,
            actor_type: FederatedActorType::Agent,
            role_in_room: FederatedRoomRole::Member,
            display_name: "Projected".into(),
            public_agent_descriptor: None,
            joined_at: String::new(),
            derived_presence: None,
            local_binding_available: None,
        }];
        assert_eq!(mention_roster(&local, Some(&access)), local);
    }

    #[test]
    fn mention_roster_uses_safe_projection_for_every_non_local_state() {
        let local = vec![part("stale-local", "Stale", RoomParticipantKind::Human)];
        for state in [
            RoomAccessState::Connecting,
            RoomAccessState::Live,
            RoomAccessState::Recovering,
            RoomAccessState::Revoked,
        ] {
            let mut access = test_access(state);
            access.members = vec![FederatedRoomMemberProjection {
                member_id: "remote-agent".into(),
                owner_member_id: None,
                actor_type: FederatedActorType::Agent,
                role_in_room: FederatedRoomRole::Member,
                display_name: "Remote Agent".into(),
                public_agent_descriptor: None,
                joined_at: String::new(),
                derived_presence: None,
                local_binding_available: None,
            }];
            let roster = mention_roster(&local, Some(&access));
            assert_eq!(
                roster,
                vec![part(
                    "remote-agent",
                    "Remote Agent",
                    RoomParticipantKind::Agent
                )]
            );
        }
        assert!(mention_roster(&local, None).is_empty());
    }

    #[test]
    fn mention_suggestions_rank_id_prefix_then_name_then_substring() {
        let roster = vec![
            part("zeta", "Ada", RoomParticipantKind::Human),
            part("flux", "Builder", RoomParticipantKind::Agent),
            part("reflux", "Other", RoomParticipantKind::Agent),
        ];
        let got = mention_suggestions(&roster, "fl");
        let ids: Vec<&str> = got.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, vec!["flux", "reflux"]);

        // Name prefix outranks substring.
        let got = mention_suggestions(&roster, "ada");
        assert_eq!(got[0].id, "zeta");
    }

    #[test]
    fn mention_suggestion_pick_uses_the_live_partial() {
        let roster = vec![
            part("ada", "Ada", RoomParticipantKind::Human),
            part("flux", "Flux", RoomParticipantKind::Agent),
        ];
        assert_eq!(
            mention_suggestion_at(&roster, "fl", 0).map(|pick| pick.id),
            Some("flux".to_string())
        );
        assert_eq!(
            mention_suggestion_at(&roster, "ad", 0).map(|pick| pick.id),
            Some("ada".to_string())
        );
    }

    #[test]
    fn mention_accept_validation_does_not_consume_keys_after_caret_moves() {
        let roster = vec![part("flux", "Flux", RoomParticipantKind::Agent)];
        let text = "@fl trailing";
        assert!(mention_accept_is_valid(
            text,
            Some(3),
            &roster,
            0,
            Some("flux")
        ));
        assert!(!mention_accept_is_valid(
            text,
            Some(11),
            &roster,
            0,
            Some("flux")
        ));
        assert!(!mention_accept_is_valid(
            text,
            None,
            &roster,
            0,
            Some("flux")
        ));
    }

    #[test]
    fn mention_accept_validation_rejects_a_stale_displayed_candidate() {
        let roster = vec![
            part("ax", "Adel", RoomParticipantKind::Human),
            part("ada", "Ada", RoomParticipantKind::Human),
        ];
        assert!(mention_accept_is_valid(
            "@ad",
            Some(3),
            &roster,
            0,
            Some("ada")
        ));
        assert!(!mention_accept_is_valid(
            "@ad",
            Some(2),
            &roster,
            0,
            Some("ada")
        ));
    }

    #[test]
    fn mention_suggestions_empty_partial_lists_roster_capped() {
        let roster: Vec<RoomParticipant> = (0..12)
            .map(|i| part(&format!("m{i}"), "M", RoomParticipantKind::Human))
            .collect();
        assert_eq!(mention_suggestions(&roster, "").len(), 8);
        assert!(mention_suggestions(&roster, "zzz").is_empty());
    }

    #[test]
    fn apply_mention_replaces_partial_and_positions_caret() {
        let (text, caret) = apply_mention("hi @fl tail", 3, 6, "flux");
        assert_eq!(text, "hi @flux tail");
        assert_eq!(caret, "hi @flux ".len());

        let (text, caret) = apply_mention("@", 0, 1, "designer");
        assert_eq!(text, "@designer ");
        assert_eq!(caret, text.len());

        let (text, caret) = apply_mention("@flux", 0, 3, "flux");
        assert_eq!(text, "@flux ");
        assert_eq!(caret, text.len());

        let (text, caret) = apply_mention("@fl\u{00a0}tail", 0, 3, "flux");
        assert_eq!(text, "@flux tail");
        assert_eq!(caret, "@flux ".len());
    }

    #[test]
    fn mention_popup_key_model() {
        assert_eq!(mention_popup_key(3, 0, "ArrowDown"), MentionKey::Move(1));
        assert_eq!(mention_popup_key(3, 0, "ArrowUp"), MentionKey::Move(2));
        assert_eq!(mention_popup_key(3, 2, "ArrowDown"), MentionKey::Move(0));
        assert_eq!(mention_popup_key(3, 1, "Enter"), MentionKey::Accept);
        assert_eq!(mention_popup_key(3, 1, "Tab"), MentionKey::Accept);
        assert_eq!(mention_popup_key(3, 1, "Escape"), MentionKey::Close);
        assert_eq!(mention_popup_key(3, 1, "a"), MentionKey::Pass);
        assert_eq!(mention_popup_key(0, 0, "Enter"), MentionKey::Pass);
    }

    #[test]
    fn utf16_byte_offset_roundtrip() {
        let s = "héllo @x";
        // 'é' is 1 UTF-16 unit but 2 bytes.
        assert_eq!(utf16_to_byte_idx(s, 2), 3);
        assert_eq!(byte_to_utf16_idx(s, 3), 2);
        assert_eq!(utf16_to_byte_idx(s, 99), s.len());
        assert_eq!(
            byte_to_utf16_idx(s, 99),
            s.chars().map(|c| c.len_utf16()).sum::<usize>()
        );
    }
}
