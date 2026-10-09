//! Workspace panel content shared by the right-side workspace pane
//! (crate::workspace) on every host: the Repo panel and the file-tree
//! helpers. Mounting is integrator-owned (crate::workspace), never done here.
//! Repo styling uses `deck-repo-*` class names in styles/deck.css.

pub mod files;
pub mod repo;
