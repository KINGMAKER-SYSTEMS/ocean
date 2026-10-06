//! Command palette — the deep-menu registry (⌘K UI, typed Command registry
//! with scope + availability predicates). One registry drives the palette,
//! the native menubar, and the header overflow.
//! Owned by the Palette workstream (feat/desktop-palette); real commands are
//! wired at integration time (app.rs), not here.

use leptos::ev::{self, KeyboardEvent};
use leptos::prelude::*;

use crate::search::fuzzy_score;

// ── Types ──────────────────────────────────────────────────────────────────

/// The scope a command belongs to — used for grouping and as a visible tag
/// in palette rows.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum CommandScope {
    Session,
    Files,
    Repo,
    Browser,
    App,
}

impl CommandScope {
    /// Human-readable label for the scope group header in the palette.
    fn label(self) -> &'static str {
        match self {
            CommandScope::Session => "Session",
            CommandScope::Files => "Files",
            CommandScope::Repo => "Repo",
            CommandScope::Browser => "Browser",
            CommandScope::App => "App",
        }
    }

    /// Stable sort order for scope groups in the palette.
    fn order(self) -> usize {
        match self {
            CommandScope::Session => 0,
            CommandScope::Files => 1,
            CommandScope::Repo => 2,
            CommandScope::Browser => 3,
            CommandScope::App => 4,
        }
    }
}

/// A single command in the deep-menu registry. One `Command` added to the
/// registry appears in the ⌘K palette, the native menubar, *and* the header
/// overflow — the integrator registers it once.
#[derive(Clone)]
pub struct Command {
    /// Stable identifier (e.g. `"new-session"`). Used for deduplication
    /// and event routing.
    pub id: &'static str,
    /// Display title shown in the palette row.
    pub title: String,
    /// Optional subtitle / supplementary text (e.g. a keyboard shortcut hint).
    pub hint: Option<String>,
    /// Scope for grouping and visual tagging.
    pub scope: CommandScope,
    /// Optional composer `/` alias (e.g. `Some("/new")`). Commands without
    /// an alias are invisible to the slash popover; the ⌘K palette still
    /// lists them. The alias is display + match text for `slash_filter`.
    pub slash: Option<&'static str>,
    /// Availability predicate evaluated at open/filter time. Host-gating
    /// (e.g. Tauri-only commands) happens here: the integrator wires a
    /// `Signal<bool>` that reflects the current host.
    pub enabled: Signal<bool>,
    /// Action to run when the command is selected and Enter is pressed
    /// (or the row is clicked).
    pub run: Callback<()>,
}

/// A cloneable, signal-backed command registry. Create one with
/// [`CommandRegistry::new`], register commands via [`register`](CommandRegistry::register),
/// and pass it to [`PaletteView`]. Registration after mount works — the
/// palette reacts when commands are added later.
///
/// # Example (integrator's app.rs)
///
/// ```ignore
/// let registry = CommandRegistry::new();
/// registry.register(Command {
///     id: "toggle-deck",
///     title: "Toggle Context Deck".into(),
///     hint: None,
///     scope: CommandScope::App,
///     slash: None,
///     enabled: Signal::derive(move || show_deck.get()),
///     run: Callback::new(move |_| show_deck.update(|v| *v = !*v)),
/// });
///
/// view! { <PaletteView registry=registry /> }
/// ```
#[derive(Clone)]
pub struct CommandRegistry {
    commands: RwSignal<Vec<Command>>,
}

impl CommandRegistry {
    /// Create an empty command registry, ready for registration.
    pub fn new() -> Self {
        Self {
            commands: RwSignal::new(Vec::new()),
        }
    }

    /// Register a command. Safe to call from any reactive scope; the palette
    /// picks up new commands on the next open or filter cycle.
    pub fn register(&self, cmd: Command) {
        self.commands.update(|cmds| cmds.push(cmd));
    }

    /// Run the enabled command identified by `id`. Returns `true` when a
    /// matching, currently-enabled command was found and its callback fired;
    /// `false` when the id is unknown or the command is registered but gated
    /// off by its `enabled` signal (a disabled command is refused, never run).
    ///
    /// This is the dispatch entry point event-driven callers share with the
    /// ⌘K palette: the native menubar emits a `menu-command` event carrying a
    /// command id, the host bridge hands the id here, and the registry routes
    /// it to the same `Callback` the palette runs. Unknown ids (a stray menu
    /// id that is not a palette command) resolve to a no-op `false`.
    pub fn run(&self, id: &str) -> bool {
        let Some(cmd) = self
            .commands
            .get_untracked()
            .into_iter()
            .find(|c| c.id == id)
        else {
            return false;
        };
        if cmd.enabled.get_untracked() {
            cmd.run.run(());
            true
        } else {
            false
        }
    }

    /// Filter commands that carry a slash alias by a subsequence match of
    /// `query` against the alias (leading `/` on either side ignored), best
    /// match first: an exact alias, then aliases the query is a prefix of,
    /// then scattered subsequence matches, each in registry order. An empty
    /// query returns every command with a slash alias in registry order.
    /// Drives the composer `/` popover.
    ///
    /// The order matters because Enter runs the first row. Unranked, `/h` ran
    /// `/thinking` (t-**h**-inking, registered earlier) instead of `/help`.
    pub fn slash_filter(&self, query: &str) -> Vec<Command> {
        let q = query.trim().trim_start_matches('/').to_lowercase();
        let mut ranked: Vec<(u8, Command)> = self
            .commands
            .get_untracked()
            .into_iter()
            .filter_map(|cmd| {
                let alias = cmd.slash?.trim_start_matches('/').to_lowercase();
                let rank = if q.is_empty() || alias == q {
                    0
                } else if alias.starts_with(&q) {
                    1
                } else if slash_subseq(&alias, &q) {
                    2
                } else {
                    return None;
                };
                Some((rank, cmd))
            })
            .collect();
        // Stable: registry order is kept within a rank.
        ranked.sort_by_key(|(rank, _)| *rank);
        ranked.into_iter().map(|(_, cmd)| cmd).collect()
    }

    /// The slash alias of the command `id`, with its slash.
    pub fn slash_alias(&self, id: &str) -> Option<&'static str> {
        self.commands
            .get_untracked()
            .iter()
            .find(|cmd| cmd.id == id)
            .and_then(|cmd| cmd.slash)
    }

    /// The command whose slash alias is exactly `name` (already lowercased,
    /// without the slash).
    fn slash_exact(&self, name: &str) -> Option<Command> {
        self.commands.get_untracked().into_iter().find(|cmd| {
            cmd.slash
                .is_some_and(|alias| alias.trim_start_matches('/').eq_ignore_ascii_case(name))
        })
    }

    /// The rows the composer popover shows for `text`, best match first.
    ///
    /// While the name is still being typed this is the ranked
    /// [`slash_filter`](Self::slash_filter) list. Once whitespace follows the
    /// name it is the one command that name spells, or nothing: the popover
    /// then shows exactly what [`resolve_slash`](Self::resolve_slash) will
    /// run, so a highlighted row is never a different command from the one
    /// Enter dispatches, and a sentence that merely starts with a slash shows
    /// no menu at all.
    pub fn slash_rows(&self, text: &str) -> Vec<Command> {
        let Some(line) = SlashLine::parse(text) else {
            return Vec::new();
        };
        match line.command_name() {
            SlashName::Command(name) if line.finished => {
                self.slash_exact(&name).into_iter().collect()
            }
            SlashName::Command(name) => self.slash_filter(&name),
            SlashName::Path | SlashName::Other => Vec::new(),
        }
    }

    /// Decide what a composer line means when it is submitted. `picked` is the
    /// popover row the user chose (highlighted on Enter or Tab, or clicked),
    /// when there is one.
    ///
    /// The line used to be matched fuzzily and then cleared whatever happened,
    /// so `/etc/hosts what does this do` was thrown away with "unknown
    /// command" and `/so what do you think` toggled the Sessions panel.
    ///
    /// - A line that does not start with `/`, or whose first word is a path
    ///   (`/etc/hosts`, `/notes.md`), is a message.
    /// - While the name is still being typed (no whitespace after it), the
    ///   picked row runs; without a pick, the best match does.
    /// - Once whitespace follows the name, only the command that name spells
    ///   exactly runs. An abbreviation followed by words is not a command:
    ///   `/s what do you think` must not toggle a panel.
    pub fn resolve_slash(&self, text: &str, picked: Option<&str>) -> SlashInput {
        let Some(line) = SlashLine::parse(text) else {
            return SlashInput::Prompt;
        };
        let name = match line.command_name() {
            SlashName::Command(name) => name,
            SlashName::Path => return SlashInput::Prompt,
            SlashName::Other => return SlashInput::Unknown,
        };
        let chosen = if line.finished {
            self.slash_exact(&name)
        } else if let Some(id) = picked {
            self.commands
                .get_untracked()
                .into_iter()
                .find(|cmd| cmd.id == id && cmd.slash.is_some())
        } else if name.is_empty() {
            None
        } else {
            self.slash_filter(&name).into_iter().next()
        };
        match chosen {
            Some(cmd) if cmd.enabled.get_untracked() => SlashInput::Command {
                id: cmd.id,
                args: line.args.to_string(),
            },
            Some(_) => SlashInput::Disabled,
            None => SlashInput::Unknown,
        }
    }
}

/// A composer line that starts with `/`, split once. The popover and the
/// dispatcher both read this, so they cannot disagree about what was typed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SlashLine<'a> {
    /// The first word after the slash, as typed.
    pub name: &'a str,
    /// The text after the name, trimmed.
    pub args: &'a str,
    /// Whitespace follows the name, so it is no longer being typed.
    pub finished: bool,
}

/// Punctuation a sentence, or a phone keyboard's double-space, leaves after a
/// command name.
const NAME_TRAILERS: [char; 6] = ['?', '!', '.', ',', ';', ':'];

/// What the first word of a `/` line is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SlashName {
    /// A command name, lowercased, trailing punctuation removed. Empty for a
    /// bare `/`.
    Command(String),
    /// A filesystem path or file name: the line is a message.
    Path,
    /// Neither: a second leading slash, an underscore, non-ASCII text.
    Other,
}

impl<'a> SlashLine<'a> {
    /// `None` unless `text` starts with `/`. A leading space is therefore
    /// always a way to send a line that begins with a slash as a message.
    pub fn parse(text: &'a str) -> Option<Self> {
        let rest = text.strip_prefix('/')?;
        Some(match rest.split_once(char::is_whitespace) {
            Some((name, args)) => Self {
                name,
                args: args.trim(),
                finished: true,
            },
            None => Self {
                name: rest,
                args: "",
                finished: false,
            },
        })
    }

    /// Classify the first word. Command names are ASCII letters, digits and
    /// hyphens.
    pub fn command_name(&self) -> SlashName {
        // A pasted comment (`// broken`, `/// doc`, `//! crate`, `/* note */`)
        // is a message: its first word is only comment punctuation, and text
        // follows. A bare `//` is still nothing.
        if self.finished
            && !self.name.is_empty()
            && self.name.chars().all(|c| matches!(c, '/' | '*' | '!'))
        {
            return SlashName::Path;
        }
        let core = self.name.trim_end_matches(NAME_TRAILERS);
        if core.is_empty() {
            // A bare `/` lists every command; `/ model` or `/..` names none.
            return if self.name.is_empty() && !self.finished {
                SlashName::Command(String::new())
            } else {
                SlashName::Other
            };
        }
        if core.starts_with('/') {
            return SlashName::Other;
        }
        let path_char =
            |c: char| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | '~' | ':');
        if core.contains('/') || (core.contains('.') && core.chars().all(path_char)) {
            return SlashName::Path;
        }
        if core.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
            SlashName::Command(core.to_ascii_lowercase())
        } else {
            SlashName::Other
        }
    }
}

/// What a submitted composer line means. See
/// [`CommandRegistry::resolve_slash`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SlashInput {
    /// Run this command with the text that follows its name.
    Command { id: &'static str, args: String },
    /// The command exists but is not available on this host.
    Disabled,
    /// It reads as a command and matches none: keep the draft and say so.
    Unknown,
    /// Not a command (a path, or no leading slash): send it as a message.
    Prompt,
}

impl Default for CommandRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// ── Fuzzy matching ─────────────────────────────────────────────────────────

/// Subsequence match for slash aliases. Case-insensitive; returns true when
/// every char of `query` appears in order inside `target`. This is only the
/// gate; [`CommandRegistry::slash_filter`] ranks what passes it.
fn slash_subseq(target: &str, query: &str) -> bool {
    let target = target.to_lowercase();
    let mut ti = target.chars().peekable();
    for qc in query.chars() {
        loop {
            match ti.next() {
                Some(tc) if tc == qc => break,
                Some(_) => continue,
                None => return false,
            }
        }
    }
    true
}

// ── Selection helpers ──────────────────────────────────────────────────────

/// Wrap-forward: `(idx + 1) % len`. Returns 0 for empty lists.
fn next_index(idx: usize, len: usize) -> usize {
    if len == 0 {
        0
    } else {
        (idx + 1) % len
    }
}

/// Wrap-backward: `(idx + len - 1) % len`. Returns 0 for empty lists.
fn prev_index(idx: usize, len: usize) -> usize {
    if len == 0 {
        0
    } else {
        (idx + len - 1) % len
    }
}

// ── Internal working type ──────────────────────────────────────────────────

#[derive(Clone)]
struct ScoredCommand {
    command: Command,
    score: f64,
}

impl PartialEq for ScoredCommand {
    fn eq(&self, other: &Self) -> bool {
        self.command.id == other.command.id && self.score == other.score
    }
}

// ── PaletteView component ──────────────────────────────────────────────────

/// The ⌘K command palette — a deep-menu overlay that opens on Cmd+K (macOS)
/// or Ctrl+K (everywhere else). Types are grouped by [`CommandScope`];
/// disabled commands are hidden. Fuzzy subsequence matching with scoring
/// keeps the most relevant commands at the top of each group.
///
/// # Keyboard
///
/// | Key          | Action                                      |
/// |-------------|---------------------------------------------|
/// | ⌘K / Ctrl+K | Open palette, focus input, reset query      |
/// | Esc          | Close palette                               |
/// | ↑ / ↓        | Move selection (wraps)                       |
/// | Enter        | Run selected command, close palette          |
/// | Backdrop     | click closes palette                         |
#[component]
pub fn PaletteView(registry: CommandRegistry, open: RwSignal<bool>) -> impl IntoView {
    let query = RwSignal::new(String::new());
    let selected_index = RwSignal::new(0usize);
    let input_ref: NodeRef<leptos::html::Input> = NodeRef::new();

    // ── Window-level keydown: Cmd+K / Ctrl+K opens the palette ──────────
    let _key_listener = window_event_listener(ev::keydown, move |e: KeyboardEvent| {
        // metaKey on macOS, ctrlKey elsewhere. We check both so Cmd+K works
        // on Mac and Ctrl+K works everywhere else.
        if !e.is_composing()
            && (e.meta_key() || e.ctrl_key())
            && !e.shift_key()
            && !e.alt_key()
            && e.key().eq_ignore_ascii_case("k")
        {
            e.prevent_default();
            query.set(String::new());
            selected_index.set(0);
            open.set(true);
        }
    });
    on_cleanup(move || _key_listener.remove());

    // ── Auto-focus the input when the palette opens ──────────────────────
    Effect::new(move |_| {
        if open.get() {
            request_animation_frame(move || {
                if let Some(el) = input_ref.get() {
                    let _ = el.focus();
                    el.select();
                }
            });
        }
    });

    // ── Filter + group + score ───────────────────────────────────────────
    // Re-derived every time `query` or `registry.commands` changes.
    let filtered_groups: Memo<Vec<(CommandScope, Vec<ScoredCommand>)>> = Memo::new(move |_| {
        let q = query.get();
        let q = q.trim();
        let cmds = registry.commands.get();

        let mut groups: Vec<(CommandScope, Vec<ScoredCommand>)> = Vec::new();

        for cmd in cmds.iter() {
            // Host/state gating: disabled commands are invisible.
            if !cmd.enabled.get() {
                continue;
            }

            let score = if q.is_empty() {
                0.0
            } else {
                match fuzzy_score(q, &cmd.title) {
                    Some(s) => s,
                    None => continue, // no match → hidden
                }
            };

            let group = groups.iter_mut().find(|(s, _)| *s == cmd.scope);
            if let Some((_, cmds)) = group {
                cmds.push(ScoredCommand {
                    command: cmd.clone(),
                    score,
                });
            } else {
                groups.push((
                    cmd.scope,
                    vec![ScoredCommand {
                        command: cmd.clone(),
                        score,
                    }],
                ));
            }
        }

        // Sort within each group by score descending (best first).
        for (_, cmds) in groups.iter_mut() {
            cmds.sort_by(|a, b| {
                b.score
                    .partial_cmp(&a.score)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
        }

        // Sort groups by scope order.
        groups.sort_by_key(|(s, _)| s.order());

        groups
    });

    // ── Flat list for index-based navigation ─────────────────────────────
    let flat_commands: Memo<Vec<ScoredCommand>> = Memo::new(move |_| {
        filtered_groups
            .get()
            .into_iter()
            .flat_map(|(_, cmds)| cmds)
            .collect()
    });

    let total_count = move || flat_commands.get().len();

    // ── Keyboard navigation inside the palette ───────────────────────────
    let on_input_keydown = move |ev: KeyboardEvent| {
        if ev.is_composing() {
            return;
        }
        match ev.key().as_str() {
            "Escape" => {
                ev.prevent_default();
                ev.stop_propagation();
                open.set(false);
            }
            "ArrowDown" => {
                ev.prevent_default();
                selected_index.update(|i| *i = next_index(*i, total_count()));
            }
            "ArrowUp" => {
                ev.prevent_default();
                selected_index.update(|i| *i = prev_index(*i, total_count()));
            }
            "Enter" => {
                ev.prevent_default();
                let idx = selected_index.get_untracked();
                let cmds = flat_commands.get_untracked();
                if let Some(sc) = cmds.get(idx) {
                    sc.command.run.run(());
                    open.set(false);
                }
            }
            _ => {}
        }
    };

    // ── View ─────────────────────────────────────────────────────────────
    view! {
        <Show when=move || open.get()>
            // Backdrop — clicking outside the panel closes the palette.
            <div class="palette-overlay" on:click=move |_| open.set(false)>
                // Panel — stop propagation so clicks inside don't close.
                <div class="palette-panel" on:click=|ev| ev.stop_propagation()>
                    <input
                        class="palette-input"
                        type="text"
                        placeholder="Type a command…"
                        node_ref=input_ref
                        prop:value=move || query.get()
                        on:input=move |ev| {
                            query.set(event_target_value(&ev));
                            selected_index.set(0);
                        }
                        on:keydown=on_input_keydown
                    />

                    <div class="palette-results">
                        <Show
                            when=move || !flat_commands.get().is_empty()
                            fallback=move || view! {
                                <div class="palette-empty">
                                    {move || {
                                        if query.get().trim().is_empty() {
                                            "No commands available"
                                        } else {
                                            "No matching commands"
                                        }
                                    }}
                                </div>
                            }
                        >
                            <For
                                each=move || filtered_groups.get()
                                key=|(scope, _)| *scope as usize
                                children=move |(scope, cmds)| {
                                    // Compute global index of each command in the
                                    // flat list so we can check selection state.
                                    let flat = flat_commands.get();
                                    view! {
                                        <div class="palette-group">
                                            <div class="palette-group-label">
                                                {scope.label()}
                                            </div>
                                            {
                                                cmds.into_iter().map(|sc| {
                                                    let cmd_id = sc.command.id;
                                                    let global_idx = flat.iter()
                                                        .position(|f| f.command.id == cmd_id)
                                                        .unwrap_or(0);
                                                    let is_selected = move || {
                                                        selected_index.get() == global_idx
                                                    };
                                                    let run = sc.command.run;
                                                    view! {
                                                        <div
                                                            class="palette-row"
                                                            class:palette-row--selected=is_selected
                                                            on:click=move |_| {
                                                                run.run(());
                                                                open.set(false);
                                                            }
                                                        >
                                                            <span class="palette-row-title">
                                                                {sc.command.title}
                                                            </span>
                                                            {if let Some(hint) = sc.command.hint {
                                                                view! {
                                                                    <span class="palette-row-hint">
                                                                        {hint}
                                                                    </span>
                                                                }.into_any()
                                                            } else {
                                                                ().into_any()
                                                            }}
                                                            <span class="palette-row-kbd">"↵"</span>
                                                        </div>
                                                    }
                                                }).collect::<Vec<_>>()
                                            }
                                        </div>
                                    }
                                }
                            />
                        </Show>
                    </div>
                </div>
            </div>
        </Show>
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── selection helpers ────────────────────────────────────────────────

    #[test]
    fn next_index_wraps_around() {
        assert_eq!(next_index(0, 3), 1);
        assert_eq!(next_index(1, 3), 2);
        assert_eq!(next_index(2, 3), 0);
    }

    #[test]
    fn prev_index_wraps_around() {
        assert_eq!(prev_index(0, 3), 2);
        assert_eq!(prev_index(2, 3), 1);
        assert_eq!(prev_index(1, 3), 0);
    }

    #[test]
    fn next_index_empty_returns_zero() {
        assert_eq!(next_index(0, 0), 0);
        assert_eq!(next_index(5, 0), 0);
    }

    #[test]
    fn prev_index_empty_returns_zero() {
        assert_eq!(prev_index(0, 0), 0);
        assert_eq!(prev_index(5, 0), 0);
    }

    #[test]
    fn prev_index_single_item() {
        assert_eq!(prev_index(0, 1), 0);
    }

    #[test]
    fn next_index_single_item() {
        assert_eq!(next_index(0, 1), 0);
    }

    #[test]
    fn palette_escape_consumes_the_event_before_closing() {
        let source = include_str!("palette.rs");
        let handler_start = source
            .find("let on_input_keydown = move |ev: KeyboardEvent|")
            .expect("palette production key handler");
        let handler = &source[handler_start..handler_start + 500];
        let escape = handler.find("\"Escape\"").expect("Escape arm");
        let prevent = handler[escape..]
            .find("ev.prevent_default()")
            .expect("palette Escape prevents default");
        let stop = handler[escape..]
            .find("ev.stop_propagation()")
            .expect("palette Escape stops propagation");
        let close = handler[escape..]
            .find("open.set(false)")
            .expect("palette Escape closes locally");
        assert!(prevent < close);
        assert!(stop < close);
    }

    // ── CommandScope ordering ────────────────────────────────────────────

    #[test]
    fn scope_order_is_stable() {
        let scopes = [
            CommandScope::Session,
            CommandScope::Files,
            CommandScope::Repo,
            CommandScope::Browser,
            CommandScope::App,
        ];
        for (i, s) in scopes.iter().enumerate() {
            assert_eq!(s.order(), i, "scope {s:?} should have order {i}");
        }
    }

    // ── CommandRegistry ──────────────────────────────────────────────────

    #[test]
    fn registry_starts_empty() {
        let registry = CommandRegistry::new();
        assert!(registry.commands.get_untracked().is_empty());
    }

    #[test]
    fn registry_register_adds_command() {
        let registry = CommandRegistry::new();
        let enabled = RwSignal::new(true);
        let cmd = Command {
            id: "test.cmd",
            title: "Test Command".into(),
            hint: None,
            scope: CommandScope::App,
            slash: None,
            enabled: Signal::from(enabled),
            run: Callback::new(|_| {}),
        };
        registry.register(cmd);
        assert_eq!(registry.commands.get_untracked().len(), 1);
    }

    #[test]
    fn run_dispatches_enabled_command_by_id() {
        let registry = CommandRegistry::new();
        let fired = RwSignal::new(0u32);
        let fired_run = fired;
        registry.register(Command {
            id: "toggle-files",
            title: "Toggle Files".into(),
            hint: None,
            scope: CommandScope::Files,
            slash: None,
            enabled: Signal::from(RwSignal::new(true)),
            run: Callback::new(move |_| fired_run.update(|n| *n += 1)),
        });

        // Known + enabled: dispatches and reports true.
        assert!(registry.run("toggle-files"));
        assert_eq!(fired.get_untracked(), 1);

        // Unknown id: no dispatch, still reports false; counter unchanged.
        assert!(!registry.run("no-such-command"));
        assert_eq!(fired.get_untracked(), 1);
    }

    #[test]
    fn run_refuses_disabled_command() {
        let registry = CommandRegistry::new();
        let fired = RwSignal::new(0u32);
        let enabled = RwSignal::new(false);
        registry.register(Command {
            id: "open-council",
            title: "Open Council".into(),
            hint: None,
            scope: CommandScope::App,
            slash: None,
            enabled: Signal::from(enabled),
            run: Callback::new(move |_| fired.update(|n| *n += 1)),
        });

        // Registered but disabled: refused — no callback, returns false.
        assert!(!registry.run("open-council"));
        assert_eq!(fired.get_untracked(), 0);

        // Flip the enabled signal: now it dispatches.
        enabled.set(true);
        assert!(registry.run("open-council"));
        assert_eq!(fired.get_untracked(), 1);
    }

    #[test]
    fn scope_labels_are_stable() {
        assert_eq!(CommandScope::Session.label(), "Session");
        assert_eq!(CommandScope::Files.label(), "Files");
        assert_eq!(CommandScope::Repo.label(), "Repo");
        assert_eq!(CommandScope::Browser.label(), "Browser");
        assert_eq!(CommandScope::App.label(), "App");
    }

    // ── slash_filter ────────────────────────────────────────────────────

    /// Build a minimal command with the given id + slash alias for filter tests.
    fn slash_filter_cmd(id: &'static str, slash: Option<&'static str>) -> Command {
        Command {
            id,
            title: id.into(),
            hint: None,
            scope: CommandScope::App,
            slash,
            enabled: Signal::derive(|| true),
            run: Callback::new(|_| {}),
        }
    }

    #[test]
    fn slash_filter_partial_query_returns_only_matching_alias() {
        let registry = CommandRegistry::new();
        registry.register(slash_filter_cmd("new-session", Some("/new")));
        registry.register(slash_filter_cmd("pick-model", Some("/model")));
        let ids: Vec<&str> = registry.slash_filter("mod").iter().map(|c| c.id).collect();
        assert_eq!(ids, vec!["pick-model"]);
    }

    #[test]
    fn slash_filter_empty_query_returns_all_aliased_in_registry_order() {
        let registry = CommandRegistry::new();
        registry.register(slash_filter_cmd("new-session", Some("/new")));
        registry.register(slash_filter_cmd("no-alias", None));
        registry.register(slash_filter_cmd("pick-model", Some("/model")));
        let ids: Vec<&str> = registry.slash_filter("").iter().map(|c| c.id).collect();
        assert_eq!(ids, vec!["new-session", "pick-model"]);
    }

    #[test]
    fn slash_filter_no_match_returns_empty() {
        let registry = CommandRegistry::new();
        registry.register(slash_filter_cmd("new-session", Some("/new")));
        registry.register(slash_filter_cmd("pick-model", Some("/model")));
        assert!(registry.slash_filter("xyz").is_empty());
    }

    #[test]
    fn slash_filter_ignores_leading_slash_on_alias() {
        let registry = CommandRegistry::new();
        registry.register(slash_filter_cmd("toggle-files", Some("/files")));
        let ids: Vec<&str> = registry.slash_filter("file").iter().map(|c| c.id).collect();
        assert_eq!(ids, vec!["toggle-files"]);
    }

    /// The composer's slash commands, with the ids and order `app.rs`
    /// registers them in.
    fn composer_registry() -> CommandRegistry {
        let registry = CommandRegistry::new();
        for (id, alias) in [
            ("open-ocean-floor", "/floor"),
            ("new-session", "/new"),
            ("toggle-files", "/files"),
            ("toggle-repo", "/repo"),
            ("toggle-browser", "/browser"),
            ("toggle-sessions", "/sessions"),
            ("toggle-rooms", "/rooms"),
            ("open-council", "/council"),
            ("workspace-toggle", "/workspace"),
            ("clear", "/clear"),
            ("model", "/model"),
            ("thinking", "/thinking"),
            ("help", "/help"),
        ] {
            registry.register(slash_filter_cmd(id, Some(alias)));
        }
        // Listed, but not runnable yet.
        registry.register(Command {
            enabled: Signal::derive(|| false),
            ..slash_filter_cmd("resume", Some("/resume"))
        });
        registry
    }

    fn first_match(registry: &CommandRegistry, query: &str) -> &'static str {
        registry.slash_filter(query)[0].id
    }

    fn row_ids(registry: &CommandRegistry, text: &str) -> Vec<&'static str> {
        registry.slash_rows(text).iter().map(|c| c.id).collect()
    }

    fn command(id: &'static str, args: &str) -> SlashInput {
        SlashInput::Command {
            id,
            args: args.into(),
        }
    }

    /// Enter runs the first row, so a prefix has to outrank a scattered match
    /// that merely comes earlier in the registry.
    #[test]
    fn slash_filter_ranks_prefixes_above_scattered_matches() {
        let registry = composer_registry();
        // Each of these used to pick the scattered match named in the comment.
        assert_eq!(first_match(&registry, "h"), "help"); // thinking
        assert_eq!(first_match(&registry, "se"), "toggle-sessions"); // browser
        assert_eq!(first_match(&registry, "cl"), "clear"); // council
        assert_eq!(first_match(&registry, "m"), "model"); // rooms
        assert_eq!(first_match(&registry, "new"), "new-session");

        // Scattered matches still appear, after the prefixes.
        let ids: Vec<&str> = registry.slash_filter("h").iter().map(|c| c.id).collect();
        assert_eq!(ids, vec!["help", "thinking"]);
    }

    #[test]
    fn slash_line_splits_the_name_from_its_arguments_once() {
        assert_eq!(SlashLine::parse("plain text"), None);
        // A leading space is the way to send a line that starts with a slash.
        assert_eq!(SlashLine::parse(" /model gpt-5"), None);
        assert_eq!(
            SlashLine::parse("/model"),
            Some(SlashLine {
                name: "model",
                args: "",
                finished: false
            })
        );
        assert_eq!(
            SlashLine::parse("/model "),
            Some(SlashLine {
                name: "model",
                args: "",
                finished: true
            })
        );
        assert_eq!(
            SlashLine::parse("/model   gpt-5.5 \n thanks "),
            Some(SlashLine {
                name: "model",
                args: "gpt-5.5 \n thanks",
                finished: true
            })
        );
        assert_eq!(
            SlashLine::parse("/ model"),
            Some(SlashLine {
                name: "",
                args: "model",
                finished: true
            })
        );
    }

    #[test]
    fn the_first_word_is_a_command_name_a_path_or_neither() {
        let name = |text: &str| SlashLine::parse(text).unwrap().command_name();
        assert_eq!(name("/Model"), SlashName::Command("model".into()));
        assert_eq!(name("/"), SlashName::Command(String::new()));
        // Punctuation a sentence or a phone keyboard leaves behind.
        assert_eq!(name("/help?"), SlashName::Command("help".into()));
        assert_eq!(name("/new. "), SlashName::Command("new".into()));
        // Paths and file names are messages.
        assert_eq!(name("/etc/hosts what is this"), SlashName::Path);
        assert_eq!(name("/notes.md"), SlashName::Path);
        assert_eq!(name("/Users/me/app.rs:12"), SlashName::Path);
        assert_eq!(name("/app.rs:12 is wrong"), SlashName::Path);
        // A pasted comment is a message.
        for text in [
            "// this function is broken\nfn foo() {}",
            "/// doc comment",
            "//! crate docs",
            "/* block */ code",
            "/** jsdoc */ code",
        ] {
            assert_eq!(name(text), SlashName::Path, "{text:?}");
        }
        // Not a name, and not a path either.
        assert_eq!(name("//model gpt-5"), SlashName::Other);
        assert_eq!(name("//"), SlashName::Other);
        assert_eq!(name("/ model"), SlashName::Other);
        assert_eq!(name("/new_session"), SlashName::Other);
        assert_eq!(name("/..."), SlashName::Other);
        assert_eq!(name("/\u{e9}t\u{e9}"), SlashName::Other);
        // A dot alone does not make a path of a line with other punctuation.
        assert_eq!(name("/model=gpt-5.5"), SlashName::Other);
        assert_eq!(name("/model=gpt-5"), SlashName::Other);
    }

    #[test]
    fn slash_alias_names_a_command_with_its_slash() {
        let registry = composer_registry();
        assert_eq!(registry.slash_alias("thinking"), Some("/thinking"));
        assert_eq!(registry.slash_alias("model"), Some("/model"));
        assert_eq!(registry.slash_alias("no-such-command"), None);
    }

    #[test]
    fn popover_rows_narrow_to_the_exact_command_once_the_name_is_finished() {
        let registry = composer_registry();
        // Still typing the name: the ranked list.
        assert_eq!(
            row_ids(&registry, "/m"),
            vec!["model", "toggle-rooms", "resume"]
        );
        assert_eq!(row_ids(&registry, "/").len(), 14);
        // Name finished: that command, or nothing.
        assert_eq!(row_ids(&registry, "/model "), vec!["model"]);
        assert_eq!(row_ids(&registry, "/MODEL gpt-5"), vec!["model"]);
        assert!(row_ids(&registry, "/m gpt-5").is_empty());
        // A sentence shows no menu, so Tab and Enter have nothing to pick.
        assert!(row_ids(&registry, "/so what do you think").is_empty());
        assert!(row_ids(&registry, "/s what do you think").is_empty());
        // Nor does a path, a doubled slash, or a line that is not a command.
        assert!(row_ids(&registry, "/etc/hosts").is_empty());
        assert!(row_ids(&registry, "//model gpt-5").is_empty());
        assert!(row_ids(&registry, "/ model").is_empty());
        assert!(row_ids(&registry, "plain").is_empty());
    }

    #[test]
    fn resolve_slash_runs_the_command_that_was_typed() {
        let registry = composer_registry();
        assert_eq!(
            registry.resolve_slash("/model openai/gpt-6", None),
            command("model", "openai/gpt-6")
        );
        assert_eq!(
            registry.resolve_slash("/Thinking high", None),
            command("thinking", "high")
        );
        // The words after a name are handed over whole; the command decides
        // what to do with them.
        assert_eq!(
            registry.resolve_slash("/help me fix this bug", None),
            command("help", "me fix this bug")
        );
        // Trailing punctuation does not turn a command into a message.
        assert_eq!(registry.resolve_slash("/help?", None), command("help", ""));
        assert_eq!(
            registry.resolve_slash("/new. ", None),
            command("new-session", "")
        );
        // No arguments yet and nothing picked: the best match, which is the
        // row the popover lists first.
        assert_eq!(registry.resolve_slash("/h", None), command("help", ""));
        assert_eq!(registry.resolve_slash("/m", None), command("model", ""));
    }

    #[test]
    fn resolve_slash_runs_the_picked_row_while_the_name_is_being_typed() {
        let registry = composer_registry();
        assert_eq!(
            registry.resolve_slash("/m", Some("toggle-rooms")),
            command("toggle-rooms", "")
        );
        assert_eq!(
            registry.resolve_slash("/", Some("toggle-files")),
            command("toggle-files", "")
        );
        // A finished name is what runs, whatever row an old highlight was on.
        assert_eq!(
            registry.resolve_slash("/model gpt-5", Some("toggle-rooms")),
            command("model", "gpt-5")
        );
        assert_eq!(
            registry.resolve_slash("/m gpt-5", Some("toggle-rooms")),
            SlashInput::Unknown
        );
        // A pick is a slash command or it is nothing.
        assert_eq!(
            registry.resolve_slash("/m", Some("no-such-command")),
            SlashInput::Unknown
        );
    }

    #[test]
    fn resolve_slash_never_runs_a_command_for_prose_or_a_path() {
        let registry = composer_registry();
        // A path is a message.
        assert_eq!(
            registry.resolve_slash("/etc/hosts what does this do", None),
            SlashInput::Prompt
        );
        assert_eq!(
            registry.resolve_slash("/notes.md", None),
            SlashInput::Prompt
        );
        assert_eq!(
            registry.resolve_slash("plain message", None),
            SlashInput::Prompt
        );
        // An abbreviation followed by words is not a command. Each of these
        // used to toggle a panel or change the model and discard the text.
        for text in [
            "/so what do you think",
            "/s what do you think",
            "/r now",
            "/m gpt-5",
            "/mod gpt-5.5",
            "/tmp is full, clean it",
        ] {
            assert_eq!(
                registry.resolve_slash(text, None),
                SlashInput::Unknown,
                "{text}"
            );
        }
        // Not names at all: kept as a draft, never sent to the model.
        for text in ["/", "/zz", "//model gpt-5", "/ model", "/new_session"] {
            assert_eq!(
                registry.resolve_slash(text, None),
                SlashInput::Unknown,
                "{text}"
            );
        }
    }

    /// The row the popover highlights is the command that runs: for every
    /// line, picking any listed row resolves to that row.
    #[test]
    fn every_listed_row_resolves_to_itself() {
        let registry = composer_registry();
        for text in [
            "/",
            "/m",
            "/mo",
            "/model",
            "/model ",
            "/model gpt-5",
            "/h",
            "/help?",
            "/r",
            "/res",
            "/resume now",
            "/so what do you think",
            "/etc/hosts",
        ] {
            let rows = registry.slash_rows(text);
            if SlashLine::parse(text).is_some_and(|line| line.finished) {
                assert!(rows.len() <= 1, "{text}");
            }
            for row in rows {
                let resolved = registry.resolve_slash(text, Some(row.id));
                if row.enabled.get_untracked() {
                    assert!(
                        matches!(resolved, SlashInput::Command { id, .. } if id == row.id),
                        "{text}: {} resolved to {resolved:?}",
                        row.id
                    );
                } else {
                    assert_eq!(resolved, SlashInput::Disabled, "{text}");
                }
            }
        }
    }

    #[test]
    fn resolve_slash_reports_a_disabled_command_as_disabled() {
        let registry = composer_registry();
        assert_eq!(
            registry.resolve_slash("/resume", None),
            SlashInput::Disabled
        );
        assert_eq!(
            registry.resolve_slash("/resume last", None),
            SlashInput::Disabled
        );
        assert_eq!(
            registry.resolve_slash("/res", Some("resume")),
            SlashInput::Disabled
        );
    }
}
