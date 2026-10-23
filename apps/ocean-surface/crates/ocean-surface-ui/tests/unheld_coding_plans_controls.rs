//! Coding-plans controls that measurement proves nothing else holds.
//!
//! Same lane and discipline as `unheld_agent_stream_reset.rs`: each control
//! was mutated for real and the wasm clippy gate run; only what stayed GREEN
//! is pinned.
//!
//! ## Measured
//!
//! | Control | Result |
//! |---|---|
//! | `app.rs` header overflow's `Coding plans` row | GREEN — pinned |
//! | `app.rs` `<CodingPlansPanel>` mount | RED — compiler-held |
//! | `coding_plans.rs` row `Sign in` click | RED — compiler-held (`sign_in` unused) |
//! | `coding_plans.rs` row `Sign out` ARMING click | RED — compiler-held (`arm_logout` unused) |
//!
//! The overflow row and the ⌘K `coding-plans` command both call
//! `CodingPlansState::show`, so deleting either one leaves `show` referenced
//! by the other and nothing warns. Deleting both is a `never used` error. The
//! overflow row is the one pinned because it is the door on a phone, where
//! there is no ⌘K.
//!
//! Needles run over production source only, so the modules' own tests cannot
//! satisfy them, with whitespace stripped so rustfmt may rewrap freely.

use std::path::Path;

/// A module read relative to the crate's `src/`.
fn src(rel: &str) -> String {
    let path = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/src")).join(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read src/{rel}: {e}"))
}

/// The half of a `src/` module that a release build actually compiles: the
/// source above its trailing `#[cfg(test)]` module.
fn view_source(rel: &str) -> String {
    src(rel)
        .split_once("#[cfg(test)]")
        .unwrap_or_else(|| panic!("src/{rel} carries its unit tests at the bottom"))
        .0
        .to_string()
}

/// Whitespace stripped wholesale, so rustfmt is free to wrap a needle however
/// it likes without breaking the test.
fn without_whitespace(source: &str) -> String {
    source
        .chars()
        .filter(|char| !char.is_whitespace())
        .collect()
}

#[test]
fn the_header_overflow_offers_a_way_into_coding_plans() {
    let view = without_whitespace(&view_source("app.rs"));
    assert!(
        view.contains("coding_plans.show();}>\"Codingplans\"</button>"),
        "the header overflow's `Coding plans` row is the phone's only door to \
         the panel; deleting it leaves `show` still called by the palette, so \
         nothing warns",
    );
}

#[test]
fn the_panel_is_absent_where_no_transport_can_carry_the_key() {
    let view = without_whitespace(&view_source("app.rs"));
    assert!(
        view.contains("<Showwhen=crate::coding_plans::supported><crate::coding_plans::CodingPlansPanelstate=coding_plans/></Show>"),
        "the panel mounts only behind `coding_plans::supported`; an extension \
         host has no proxy session and no native key custody, so every request \
         there would be refused",
    );
    assert!(
        view.contains("<Showwhen=crate::coding_plans::supported><buttonclass=\"ocean-more__item\""),
        "the header overflow row is gated the same way as the panel",
    );
}
