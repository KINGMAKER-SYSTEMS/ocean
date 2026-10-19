//! Center-rail notices: pending invitation, access state, share-link result,
//! and the federation outbox.

use leptos::prelude::*;

use crate::rooms::{OutboxItemState, RoomAccessState, Rooms};

/// "Join shared room?" prompt for an invitation link opened in this Ocean.
#[component]
pub(super) fn PendingInviteBanner(rooms: Rooms) -> impl IntoView {
    move || {
        rooms.pending_invite.get().map(|_| {
        view! {
            <div class="rooms-workspace__pending-invite" role="dialog" aria-label="Room invitation">
                <div class="rooms-workspace__pending-invite-copy">
                    <strong>"Join shared room?"</strong>
                    <span>"Review and join it through this Ocean."</span>
                </div>
                {move || {
                    let status = rooms.status.get();
                    (!status.is_empty()).then(|| view! {
                        <div class="rooms-workspace__status" role="status">{status}</div>
                    })
                }}
                <button
                    class="rooms-workspace__invite-button"
                    type="button"
                    disabled=move || rooms.pending_invite_joining()
                    on:click=move |_| {
                        if let Some(invite) = rooms.pending_invite.get_untracked() {
                            rooms.redeem_invite(invite.room_key, invite.code);
                        }
                    }
                >
                    "Join room"
                </button>
                <button
                    class="rooms-workspace__pending-invite-dismiss"
                    type="button"
                    on:click=move |_| rooms.dismiss_pending_invite()
                >
                    "Not now"
                </button>
            </div>
        }
    })
    }
}

/// Access status banner (Connecting, Recovering, Revoked).
#[component]
pub(super) fn AccessStateBanner(rooms: Rooms) -> impl IntoView {
    move || {
        let state = rooms.access.get().map(|a| a.state);
        match state {
            Some(RoomAccessState::Connecting) => view! {
                <div
                    class="room-stage__access-state room-stage__access-state--connecting"
                    role="status"
                    aria-live="polite"
                >
                    "Connecting to federated room…"
                </div>
            }
            .into_any(),
            Some(RoomAccessState::Recovering) => view! {
                <div
                    class="room-stage__access-state room-stage__access-state--recovering"
                    role="status"
                    aria-live="polite"
                >
                    "Recovering connection…"
                </div>
            }
            .into_any(),
            Some(RoomAccessState::Revoked) => view! {
                <div
                    class="room-stage__access-state room-stage__access-state--revoked"
                    role="alert"
                >
                    "Access revoked"
                </div>
            }
            .into_any(),
            _ => ().into_any(),
        }
    }
}

/// Explicit invite result. Codes appear only after an owner asks to share and
/// are copied as an `ocean://` link for the installed app. The copy state is
/// owned by `RoomsWorkspace` so it survives the center rail re-rendering.
#[component]
pub(super) fn InviteResult(
    rooms: Rooms,
    copied_invite_link: RwSignal<Option<String>>,
    invite_copy_error: RwSignal<Option<(String, String)>>,
) -> impl IntoView {
    move || {
        if let Some(invite) = rooms.invite.get() {
            let link = format!("ocean://room/{}/join?code={}", invite.room_key, invite.code);
            let copy_link = link.clone();
            let copied_state_link = link.clone();
            let error_state_link = link.clone();
            view! {
                <div class="rooms-workspace__invite" role="status">
                    <div class="rooms-workspace__invite-copy">
                        <span class="rooms-workspace__invite-label">
                            {format!("Invite expires {}", invite.expires_at)}
                        </span>
                        <code class="rooms-workspace__invite-link">{link}</code>
                    </div>
                    <button
                        class="rooms-workspace__invite-button"
                        type="button"
                        on:click=move |_| {
                            copied_invite_link.set(None);
                            invite_copy_error.set(None);
                            let copied_link = copy_link.clone();
                            let failed_link = copy_link.clone();
                            if let Some(window) = web_sys::window() {
                                let promise = window
                                    .navigator()
                                    .clipboard()
                                    .write_text(&copied_link);
                                wasm_bindgen_futures::spawn_local(async move {
                                    match wasm_bindgen_futures::JsFuture::from(promise).await {
                                        Ok(_) => copied_invite_link.set(Some(copied_link)),
                                        Err(_) => invite_copy_error.set(Some((
                                            failed_link,
                                            "Copy failed—select the link manually.".into(),
                                        ))),
                                    }
                                });
                            } else {
                                invite_copy_error.set(Some((
                                    failed_link,
                                    "Copy failed—select the link manually.".into(),
                                )));
                            }
                        }
                    >
                        {move || if copied_invite_link.get().as_deref()
                            == Some(copied_state_link.as_str())
                        {
                            "Copied"
                        } else {
                            "Copy link"
                        }}
                    </button>
                    {move || invite_copy_error.get().and_then(|(link, error)| {
                        (link == error_state_link).then(|| view! {
                            <span class="rooms-workspace__invite-label" role="alert">{error}</span>
                        })
                    })}
                </div>
            }
            .into_any()
        } else if let Some(error) = rooms.invite_error.get() {
            view! {
                <div class="rooms-workspace__invite rooms-workspace__invite--error" role="alert">
                    {error}
                </div>
            }
            .into_any()
        } else {
            ().into_any()
        }
    }
}

/// Federation outbox, explicitly outside the confirmed transcript. Pending
/// items are informational; only failed items can retry.
#[component]
pub(super) fn OutboxPanel(rooms: Rooms) -> impl IntoView {
    move || {
        let outbox = rooms
            .access
            .get()
            .map(|access| access.outbox)
            .unwrap_or_default();
        if outbox.is_empty() {
            ().into_any()
        } else {
            view! {
                <div
                    class="rooms-workspace__outbox"
                    aria-label="Messages awaiting federation"
                    aria-live="polite"
                >
                    <For
                        each=move || rooms.access.get()
                            .map(|access| access.outbox)
                            .unwrap_or_default()
                        key=|item| item.client_event_id.clone()
                        children=move |item| {
                            let failed = item.state == OutboxItemState::Failed;
                            let body = item.payload.get("body")
                                .and_then(|body| body.as_str())
                                .unwrap_or("Message awaiting confirmation")
                                .to_string();
                            let event_id = item.client_event_id.clone();
                            view! {
                                <div
                                    class="rooms-workspace__outbox-item"
                                    class:rooms-workspace__outbox-item--failed=failed
                                >
                                    <span class="rooms-workspace__outbox-state">
                                        {if failed { "Failed" } else { "Pending" }}
                                    </span>
                                    <span class="rooms-workspace__outbox-body">
                                        {body}
                                    </span>
                                    {if failed {
                                        view! {
                                            <button
                                                class="rooms-workspace__outbox-retry"
                                                type="button"
                                                aria-label="Retry failed message"
                                                on:click=move |_| rooms.retry_outbox(event_id.clone())
                                            >
                                                "Retry"
                                            </button>
                                        }.into_any()
                                    } else {
                                        ().into_any()
                                    }}
                                </div>
                            }
                        }
                    />
                </div>
            }.into_any()
        }
    }
}
