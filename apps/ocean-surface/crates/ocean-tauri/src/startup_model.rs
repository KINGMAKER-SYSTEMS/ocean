//! Desktop startup intent. This non-secret fallback never changes daemon state.
use std::io::{Read, Write};
use std::path::Path;

use serde::Serialize;

#[derive(Serialize)]
pub(crate) struct ModelChoice {
    id: String,
    label: String,
}

#[derive(Serialize)]
pub(crate) struct Setup {
    models: Vec<ModelChoice>,
    selected: Option<String>,
    needs_model: bool,
}

pub(crate) fn validate(model: &str) -> Result<(), String> {
    if ocean_providers::model_routes()
        .iter()
        .any(|route| route.id == model)
    {
        Ok(())
    } else {
        Err("Choose a model from the list.".into())
    }
}

pub(crate) fn load(path: &Path) -> Result<Option<String>, String> {
    match path.symlink_metadata() {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Ok(metadata) if metadata.is_file() && metadata.len() <= 512 => {}
        _ => return Err("Couldn't read the startup model setting.".into()),
    }
    let mut model = String::new();
    std::fs::File::open(path)
        .and_then(|file| file.take(513).read_to_string(&mut model))
        .map_err(|_| "Couldn't read the startup model setting.")?;
    // Preserve a previously selected route when a newer catalog retires it.
    // The daemon alone decides whether that saved route still resolves.
    let segment = |value: &str| {
        !value.is_empty()
            && value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    };
    if !model
        .split_once('/')
        .is_some_and(|(provider, id)| segment(provider) && segment(id))
    {
        return Err("Couldn't read the startup model setting.".into());
    }
    Ok(Some(model))
}

pub(crate) fn save(path: &Path, model: &str) -> Result<(), String> {
    validate(model)?;
    // Refuse surprising destinations instead of replacing a link or directory.
    load(path)?;
    let parent = path
        .parent()
        .ok_or("Couldn't save the startup model setting.")?;
    std::fs::create_dir_all(parent).map_err(|_| "Couldn't save the startup model setting.")?;
    let temporary = parent.join(format!(".startup-model-{}", uuid::Uuid::new_v4()));
    let result = (|| -> std::io::Result<()> {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temporary)?;
        file.write_all(model.as_bytes())?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&temporary, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result.map_err(|_| "Couldn't save the startup model setting.".into())
}

pub(crate) fn setup(path: &Path) -> Result<Setup, String> {
    let selected = load(path)?;
    let configured = ["OCEAN_MODEL", "OCEAN_DEFAULT_MODEL"]
        .iter()
        .any(|key| std::env::var(key).is_ok_and(|value| !value.trim().is_empty()))
        || ocean_providers::default_config_dir()
            .is_some_and(|path| path.join("last_model").is_file());
    Ok(Setup {
        needs_model: selected.is_none() && !configured,
        selected,
        models: ocean_providers::model_routes()
            .into_iter()
            .map(|route| ModelChoice {
                id: route.id,
                label: route.label,
            })
            .collect(),
    })
}

pub(crate) fn apply(command: &mut std::process::Command, model: Option<&str>) {
    apply_with_environment(
        command,
        model,
        std::env::var("OCEAN_MODEL").ok().as_deref(),
        std::env::var("OCEAN_DEFAULT_MODEL").ok().as_deref(),
    );
}

fn apply_with_environment(
    command: &mut std::process::Command,
    model: Option<&str>,
    explicit: Option<&str>,
    default: Option<&str>,
) {
    // Blank explicit values must not shadow a persisted/default daemon choice,
    // including when the desktop has no saved selection of its own.
    if explicit.is_some_and(|value| value.trim().is_empty()) {
        command.env_remove("OCEAN_MODEL");
    }
    // This is a fallback only. The daemon's explicit and persisted choices win.
    if let Some(model) = model {
        if default.is_none_or(|value| value.trim().is_empty()) {
            command.env("OCEAN_DEFAULT_MODEL", model);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn startup_model_is_only_a_child_fallback_and_blank_overrides_do_not_block_it() {
        let mut command = std::process::Command::new("inert-fixture");
        apply_with_environment(
            &mut command,
            Some("openai/gpt-4o"),
            Some("explicit"),
            Some("configured"),
        );
        assert_eq!(command.get_envs().count(), 0);
        apply_with_environment(&mut command, Some("openai/gpt-4o"), Some(" "), Some(""));
        let values: std::collections::BTreeMap<_, _> = command.get_envs().collect();
        assert_eq!(values[std::ffi::OsStr::new("OCEAN_MODEL")], None);
        assert_eq!(
            values[std::ffi::OsStr::new("OCEAN_DEFAULT_MODEL")],
            Some(std::ffi::OsStr::new("openai/gpt-4o"))
        );
    }

    #[test]
    fn startup_model_without_desktop_selection_preserves_daemon_precedence() {
        for explicit in [None, Some(""), Some(" \t\n"), Some("openai/gpt-4o")] {
            for default in [None, Some(""), Some("openai/gpt-4o-mini")] {
                let mut command = std::process::Command::new("inert-fixture");
                apply_with_environment(&mut command, None, explicit, default);
                let values: std::collections::BTreeMap<_, _> = command.get_envs().collect();
                if explicit.is_some_and(|value| value.trim().is_empty()) {
                    assert_eq!(values.len(), 1);
                    assert_eq!(values[std::ffi::OsStr::new("OCEAN_MODEL")], None);
                } else {
                    assert!(values.is_empty());
                }
                // No desktop choice may manufacture/replace a daemon default.
                assert!(!values.contains_key(std::ffi::OsStr::new("OCEAN_DEFAULT_MODEL")));
            }
        }
    }

    #[test]
    fn startup_model_round_trips_and_replaces_only_its_setting() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("desktop/startup-model");
        assert_eq!(load(&path).unwrap(), None);
        let models = ocean_providers::model_routes();
        save(&path, &models[0].id).unwrap();
        assert_eq!(load(&path).unwrap().as_deref(), Some(models[0].id.as_str()));
        save(&path, &models[1].id).unwrap();
        assert_eq!(load(&path).unwrap().as_deref(), Some(models[1].id.as_str()));
        assert_eq!(
            std::fs::read_dir(path.parent().unwrap()).unwrap().count(),
            1
        );
    }

    #[test]
    fn startup_model_invalid_intent_preserves_the_previous_choice() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("startup-model");
        let valid = ocean_providers::model_routes()[0].id.clone();
        save(&path, &valid).unwrap();
        for value in [
            "",
            "fake-ok",
            "../../bin/program",
            "openai-chatgpt/unlisted",
            "--help",
        ] {
            assert!(save(&path, value).is_err());
            assert_eq!(load(&path).unwrap(), Some(valid.clone()));
        }
    }

    #[test]
    fn startup_model_retired_choice_stays_owned_by_the_daemon_resolver() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("startup-model");
        std::fs::write(&path, "openai/retired-model").unwrap();
        assert_eq!(
            load(&path).unwrap().as_deref(),
            Some("openai/retired-model")
        );
        assert!(validate("openai/retired-model").is_err());
    }

    #[test]
    fn startup_model_corrupt_or_nonregular_setting_is_not_replaced() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("startup-model");
        std::fs::write(&path, "x".repeat(513)).unwrap();
        assert!(load(&path).is_err());
        let valid = ocean_providers::model_routes()[0].id.clone();
        assert!(save(&path, &valid).is_err());
        assert_eq!(std::fs::metadata(&path).unwrap().len(), 513);
        assert!(save(root.path(), &valid).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn startup_model_symlink_is_not_replaced() {
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("target");
        std::fs::write(&target, "preserve").unwrap();
        let path = root.path().join("startup-model");
        std::os::unix::fs::symlink(&target, &path).unwrap();
        assert!(save(&path, &ocean_providers::model_routes()[0].id).is_err());
        assert_eq!(std::fs::read_to_string(target).unwrap(), "preserve");
    }
}
