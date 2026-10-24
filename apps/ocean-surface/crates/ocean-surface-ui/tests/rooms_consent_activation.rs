//! Rooms consent — the activation choice in the consent sheet.
//!
//! Source guard (same style as rooms_agent_badge.rs / rooms_product_doc_truth.rs)
//! for the one trust decision the design made in
//! `docs/OCEAN_ROOMS_DESIGN_DIRECTION.md` §5.1 step 3 — "three radio groups
//! with the daemon defaults selected and nothing else pre-ticked: activation
//! explicit_only / mention / task_and_thread" — so the three sources that
//! name an activation policy cannot drift apart:
//!   1. the daemon's closed set (`room_agent_authority::parse_activation`, in
//!      the monorepo), the shell's `room_consent::Activation::parse` and the
//!      page's `rooms::ConsentActivation::parse` accept exactly the same wire
//!      names, in the same order;
//!   2. all three default to `explicit_only` (the daemon's
//!      `default_activation_policy`, the shell's `Activation::DEFAULT`, the
//!      page's `#[default]`), so a fresh authorization never opens wider;
//!   3. the sheet renders one radio per `ConsentActivation::ALL` entry and
//!      the choice travels page → `host::room_consent_authorize` →
//!      the shell command's `activation_policy` → the frozen body, with the
//!      native dialog naming it;
//!   4. the deciding design text and the product doc still say so.

use std::path::Path;

const DESIGN_DOC: &str = "docs/OCEAN_ROOMS_DESIGN_DIRECTION.md";
const PRODUCT_DOC: &str = "docs/OCEAN_ROOMS_PRODUCT.md";
const SHELL: &str = "crates/ocean-tauri/src/room_consent.rs";
const DAEMON_AUTHORITY: &str =
    "../../platform/ocean-os/crates/ocean-daemon/src/room_agent_authority.rs";

/// The policies the design names, in the design's order.
const POLICIES: [&str; 3] = ["explicit_only", "mention", "task_and_thread"];

fn repo_root() -> &'static Path {
    // tests/ -> ocean-surface-ui/ -> crates/ -> Surface component root
    Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
}

fn read(rel: &str) -> String {
    let path = repo_root().join(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

fn page(rel: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

/// The production half of a module: everything above its trailing
/// `#[cfg(test)] mod tests`, so a needle the tests quote never satisfies a scan.
fn production(source: &str) -> &str {
    source
        .rsplit_once("#[cfg(test)]\nmod tests {")
        .or_else(|| source.rsplit_once("#[cfg(all(test, any(unix, windows)))]\nmod tests {"))
        .map_or(source, |(production, _)| production)
}

/// Whitespace-free view so multi-line `view!` markup matches as one string.
fn squash(input: &str) -> String {
    input.chars().filter(|c| !c.is_whitespace()).collect()
}

/// The string literals of the `"…" =>` arms of the first `match` after
/// `anchor` then `fn_sig`, up to its wildcard arm.
fn match_arm_literals(src: &str, anchor: &str, fn_sig: &str) -> Vec<String> {
    let at = src
        .find(anchor)
        .unwrap_or_else(|| panic!("no `{anchor}` in source"));
    let rest = &src[at..];
    let at = rest
        .find(fn_sig)
        .unwrap_or_else(|| panic!("no `{fn_sig}` after `{anchor}`"));
    let rest = &rest[at..];
    let open = rest.find("match ").expect("fn body opens a match");
    let rest = &rest[open..];
    let end = rest.find("_ =>").expect("match ends in a wildcard arm");
    rest[..end]
        .lines()
        .filter_map(|line| {
            let lit = line.trim().strip_prefix('"')?;
            let (name, after) = lit.split_once('"')?;
            after
                .trim_start()
                .starts_with("=>")
                .then(|| name.to_string())
        })
        .collect()
}

#[test]
fn page_shell_and_daemon_accept_exactly_the_same_three_activation_names() {
    let rooms = page("src/rooms.rs");
    let page_arms = match_arm_literals(
        production(&rooms),
        "impl ConsentActivation {",
        "fn parse(raw: &str)",
    );
    assert_eq!(page_arms, POLICIES, "rooms::ConsentActivation::parse");

    let shell = read(SHELL);
    let shell_arms = match_arm_literals(
        production(&shell),
        "impl Activation {",
        "fn parse(raw: &str)",
    );
    assert_eq!(shell_arms, POLICIES, "room_consent::Activation::parse");

    // The page's wire names and the shell's are the same literals too.
    for policy in POLICIES {
        assert!(
            production(&rooms).contains(&format!("=> \"{policy}\"")),
            "ConsentActivation::wire lacks `{policy}`"
        );
        assert!(
            production(&shell).contains(&format!("=> \"{policy}\"")),
            "Activation::as_str lacks `{policy}`"
        );
    }

    // In the monorepo the daemon is the authority on the set; standalone
    // Surface checkouts skip this half (the page/shell halves still hold).
    let daemon = repo_root().join(DAEMON_AUTHORITY);
    if daemon.is_file() {
        let daemon = std::fs::read_to_string(&daemon).expect("daemon authority source");
        let daemon_arms = match_arm_literals(
            production(&daemon),
            "fn parse_activation(raw: &str)",
            "fn parse_activation(raw: &str)",
        );
        assert_eq!(
            daemon_arms, POLICIES,
            "room_agent_authority::parse_activation"
        );
        assert!(
            production(&daemon).contains(
                "fn default_activation_policy() -> String {\n    \"explicit_only\".into()"
            ),
            "the daemon's default_activation_policy is no longer `explicit_only`; the consent \
             sheet's pre-selection and this guard must follow it together",
        );
    }
}

#[test]
fn every_layer_defaults_to_explicit_only_so_a_fresh_authorization_never_opens_wider() {
    let rooms = squash(production(&page("src/rooms.rs")));
    assert!(
        rooms.contains("pubenumConsentActivation{///Runsonlythroughtheexplicitinvokemenu.#[default]ExplicitOnly,"),
        "ConsentActivation's #[default] must be ExplicitOnly",
    );
    assert!(
        rooms.contains(
            "pubconstALL:[Self;3]=[Self::ExplicitOnly,Self::Mention,Self::TaskAndThread];"
        ),
        "ConsentActivation::ALL must list the three policies in the design's order",
    );
    // The draft opens on what the shell froze and falls back to the default,
    // never to a wider policy.
    assert!(rooms.contains(".and_then(ConsentActivation::parse).unwrap_or_default();"));

    let shell = squash(production(&read(SHELL)));
    assert!(
        shell.contains("pub(crate)constDEFAULT:Self=Self::ExplicitOnly;"),
        "room_consent::Activation::DEFAULT must be ExplicitOnly",
    );
    // A new authorization (no binding) freezes the default; a binding's own
    // policy is parsed, not copied as text.
    assert!(shell.contains(
        "None|Some(Value::Null)=>Activation::DEFAULT,Some(Value::String(raw))=>Activation::parse(raw).ok_or(BAD)?,"
    ));
}

#[test]
fn the_sheet_renders_one_radio_per_policy_and_the_choice_reaches_the_frozen_body() {
    // 1. The sheet: radios from ALL, value = wire name, pre-checked from the
    //    draft, each change goes through Rooms::set_consent_activation.
    let agents = squash(production(&page("src/rooms_workspace/agents.rs")));
    assert!(agents
        .contains(r#"<fieldsetclass="rooms-workspace__consent-activation"disabled=!editable>"#));
    assert!(agents.contains("{ConsentActivation::ALL.into_iter().map(|activation|view!{"));
    assert!(agents.contains(r#"<inputtype="radio"name=CONSENT_ACTIVATION_GROUPvalue=activation.wire()checked=activation==chosenon:change=move|_|rooms.set_consent_activation(activation)/>"#));
    assert!(agents.contains(
        r#"<spanclass="rooms-workspace__consent-activation-label">{activation.label()}</span>"#
    ));
    assert!(agents.contains(
        r#"<spanclass="rooms-workspace__consent-activation-note">{activation.consequence()}</span>"#
    ));
    assert_eq!(
        agents.matches(r#"type="radio""#).count(),
        1,
        "one radio template, mapped over ALL — never hand-listed options"
    );

    // 2. The page sends the draft's choice by wire name, nothing else.
    let rooms = squash(production(&page("src/rooms.rs")));
    assert!(rooms.contains(
        "crate::host::room_consent_authorize(&draft.preview.consent_id,draft.activation.wire(),)"
    ));
    let host = squash(production(&page("src/host.rs")));
    assert!(host.contains(
        r#"letargs=consent_args(&[("consentId",consent_id),("activationPolicy",activation_policy),]);Some(matchtauri_invoke("room_consent_authorize",&args)"#
    ));

    // 3. The shell parses it natively before the dialog, the dialog names it,
    //    and the frozen body carries the enum's wire name.
    let shell_src = read(SHELL);
    let shell = squash(production(&shell_src));
    assert!(shell.contains(
        "consent_id:String,activation_policy:Option<String>,)->Result<ConsentOutcome,String>{main_window(&window)?;letactivation=parse_activation_arg(activation_policy.as_deref())?;"
    ));
    assert!(shell.contains("letfrozen=broker.begin_confirm(&consent_id,activation)?;"));
    assert!(shell_src.contains("Activation: {} ({})"));
    assert!(shell.contains(
        r#"map.insert("activation_policy".into(),json!(self.activation_policy.as_str()),);"#
    ));
    // A dispatched decision is replayed only unchanged.
    assert!(shell.contains(
        "ifentry.dispatched{entries.remove(index);returnErr(\"consent_invalidated\".into());}"
    ));
}

#[test]
fn the_deciding_design_text_and_the_product_doc_still_say_so() {
    let design = read(DESIGN_DOC);
    assert!(
        design.contains("three radio groups with the daemon defaults selected and\n   nothing else pre-ticked: activation explicit_only / mention /\n   task_and_thread"),
        "{DESIGN_DOC} §5.1 step 3 no longer specifies the activation radios with the daemon \
         default selected; if the ruling changed, change the sheet and this guard together",
    );
    assert!(design.contains(
        r#"activation label ("only when
invoked" / "when @mentioned" / "mentions, tasks and thread replies")"#
    ));

    let product = read(PRODUCT_DOC);
    for policy in POLICIES {
        assert!(
            product.contains(&format!("`{policy}`")),
            "{PRODUCT_DOC} no longer names `{policy}`"
        );
    }
    assert!(
        product.contains("pre-selected"),
        "{PRODUCT_DOC} must say which activation the consent sheet pre-selects"
    );
    assert!(
        !product.contains("binding minted with the consent default"),
        "{PRODUCT_DOC} still describes consent as minting a fixed default"
    );
}
