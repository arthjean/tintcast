use std::collections::BTreeMap;
use std::path::Path;

use serde::Serialize;
use serde_json::{Value, json};
use tintcast_engine::{SystemServices, sha256_hex};

use crate::desktop::Desktop;
use crate::keys::WATCHED;
use crate::terminal::{fragment_path, settings_candidates};

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Report {
    pub build: String,
    pub watched: BTreeMap<String, Value>,
    pub info: BTreeMap<String, Value>,
}

fn error(message: impl std::fmt::Display) -> Value {
    json!({ "error": message.to_string() })
}

fn file_digest(services: &dyn SystemServices, path: &Path) -> Value {
    match services.read_file(path) {
        Ok(Some(bytes)) => json!(sha256_hex(&bytes)),
        Ok(None) => Value::Null,
        Err(e) => error(e),
    }
}

pub fn report(services: &dyn SystemServices, desktop: &dyn Desktop) -> Report {
    let mut watched = BTreeMap::new();
    let mut info = BTreeMap::new();
    for (key, name) in WATCHED {
        let value = match services.read_registry(key, name) {
            Ok(value) => serde_json::to_value(value).unwrap_or_else(error),
            Err(e) => error(e),
        };
        watched.insert(format!(r"reg:HKCU\{key}\{name}"), value);
    }
    match services.monitors() {
        Ok(monitors) => {
            info.insert("monitors".into(), json!(monitors.len()));
            for monitor in monitors {
                let value = match services.wallpaper(&monitor) {
                    Ok(path) => json!(path.map(|p| p.display().to_string())),
                    Err(e) => error(e),
                };
                watched.insert(format!("wallpaper:{monitor}"), value);
            }
        }
        Err(e) => {
            watched.insert("wallpaper".into(), error(e));
        }
    }
    watched.insert(
        "wallpaper-position".into(),
        match services.wallpaper_position() {
            Ok(position) => json!(position),
            Err(e) => error(e),
        },
    );
    watched.insert(
        "ui-accent".into(),
        match desktop.accent_color() {
            Ok(color) => json!(color.hex()),
            Err(e) => error(e),
        },
    );
    if let Some(local) = desktop.local_app_data() {
        for (label, path) in settings_candidates(&local) {
            watched.insert(
                format!("terminal-settings:{label}"),
                file_digest(services, &path),
            );
        }
        watched.insert(
            "terminal-fragment".into(),
            file_digest(services, &fragment_path(&local)),
        );
    }
    info.insert("background-kind".into(), json!(desktop.background_kind()));
    info.insert("contrast-theme".into(), json!(desktop.contrast_theme_on()));
    info.insert(
        "wallpaper-policy".into(),
        json!(desktop.wallpaper_policy_blocks()),
    );
    info.insert(
        "dwm-colorization".into(),
        json!(desktop.dwm_colorization().map(|c| format!("0x{c:08x}"))),
    );
    match desktop.ui_colors() {
        Ok((background, foreground)) => {
            info.insert("ui-background".into(), json!(background.hex()));
            info.insert("ui-foreground".into(), json!(foreground.hex()));
        }
        Err(e) => {
            info.insert("ui-colors".into(), error(e));
        }
    }
    Report {
        build: desktop.windows_build(),
        watched,
        info,
    }
}
