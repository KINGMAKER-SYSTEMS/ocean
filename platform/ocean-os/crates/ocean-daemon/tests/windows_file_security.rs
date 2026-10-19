//! Native Windows fixtures include the actual production file-security modules.
#![cfg(windows)]

#[allow(dead_code)]
#[path = "../src/windows_skill_file.rs"]
mod windows_skill_file;

#[allow(dead_code)]
#[path = "../src/room_operator.rs"]
mod room_operator;
