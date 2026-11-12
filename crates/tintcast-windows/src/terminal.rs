use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

use serde_json::{Map, Value, json};
use tintcast_engine::jsonc::{self, JsoncError};
use tintcast_engine::{
    ApplyRequest, Detection, Integration, Operation, Palette, Plan, ReloadResult, SystemServices,
};

use crate::desktop::Desktop;

pub const ID: &str = "terminal";
pub const NOT_INSTALLED: &str = "Windows Terminal is not installed.";
pub const DEFAULT_SCHEME: &str = "/profiles/defaults/colorScheme";
pub const WINDOWS_POWERSHELL: &str = "{61c54bbd-c2c6-5271-96e7-009a87ff44bf}";
pub const COMMAND_PROMPT: &str = "{0caa0dad-35be-5f56-a8ff-afceeeaa6101}";
const ANSI_KEYS: [&str; 16] = [
    "black",
    "red",
    "green",
    "yellow",
    "blue",
    "purple",
    "cyan",
    "white",
    "brightBlack",
    "brightRed",
    "brightGreen",
    "brightYellow",
    "brightBlue",
    "brightPurple",
    "brightCyan",
    "brightWhite",
];

pub fn fragment_dir(local_app_data: &Path) -> PathBuf {
    local_app_data.join(r"Microsoft\Windows Terminal\Fragments\Tintcast")
}

pub fn fragment_path(local_app_data: &Path) -> PathBuf {
    fragment_dir(local_app_data).join("tintcast.json")
}

pub fn settings_candidates(local_app_data: &Path) -> Vec<(&'static str, PathBuf)> {
    vec![
        (
            "Windows Terminal",
            local_app_data
                .join(r"Packages\Microsoft.WindowsTerminal_8wekyb3d8bbwe\LocalState\settings.json"),
        ),
        (
            "Windows Terminal Preview",
            local_app_data.join(
                r"Packages\Microsoft.WindowsTerminalPreview_8wekyb3d8bbwe\LocalState\settings.json",
            ),
        ),
        (
            "Windows Terminal (unpackaged)",
            local_app_data.join(r"Microsoft\Windows Terminal\settings.json"),
        ),
    ]
}

pub fn scheme_name(theme_name: &str) -> String {
    format!("Tintcast {theme_name}")
}

pub fn scheme(name: &str, palette: &Palette) -> Value {
    let mut scheme = Map::new();
    scheme.insert("name".into(), json!(name));
    scheme.insert("background".into(), json!(palette.background.hex()));
    scheme.insert("foreground".into(), json!(palette.foreground.hex()));
    scheme.insert("cursorColor".into(), json!(palette.accent.hex()));
    scheme.insert("selectionBackground".into(), json!(palette.selection.hex()));
    for (key, color) in ANSI_KEYS.iter().zip(palette.ansi) {
        scheme.insert((*key).into(), json!(color.hex()));
    }
    Value::Object(scheme)
}

pub fn syntax_message(error: &JsoncError) -> String {
    match error {
        JsoncError::Syntax { line, column, .. } => format!(
            "Windows Terminal's settings.json has a syntax error at line {line}, column {column}."
        ),
        other => format!("Windows Terminal's settings.json can't be read: {other}"),
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Analysis {
    pub defaults_scheme: bool,
    pub generated: Vec<String>,
    pub keeps_own: Vec<String>,
    pub hand_written: Vec<String>,
    pub list_form: bool,
}

impl Analysis {
    pub fn needs_edit(&self) -> bool {
        self.defaults_scheme || !self.hand_written.is_empty()
    }
}

fn text(value: Option<&Value>) -> Option<&str> {
    value.and_then(Value::as_str)
}

fn is_known_guid(guid: &str) -> bool {
    guid.eq_ignore_ascii_case(WINDOWS_POWERSHELL) || guid.eq_ignore_ascii_case(COMMAND_PROMPT)
}

pub fn analyze(settings: &Value) -> Analysis {
    let mut analysis = Analysis::default();
    let profiles = settings.get("profiles");
    let list = match profiles {
        Some(Value::Array(list)) => {
            analysis.list_form = true;
            list.as_slice()
        }
        Some(Value::Object(map)) => {
            analysis.defaults_scheme = map
                .get("defaults")
                .and_then(|d| d.get("colorScheme"))
                .is_some_and(|v| !v.is_null());
            map.get("list")
                .and_then(Value::as_array)
                .map_or(&[][..], Vec::as_slice)
        }
        _ => &[][..],
    };
    for profile in list {
        if profile.get("hidden").and_then(Value::as_bool) == Some(true) {
            continue;
        }
        let guid = text(profile.get("guid"));
        let name = text(profile.get("name"))
            .or(guid)
            .unwrap_or("A profile")
            .to_owned();
        if profile.get("colorScheme").is_some_and(|v| !v.is_null()) {
            analysis.keeps_own.push(name);
        } else if profile.get("source").is_some() {
            if let Some(guid) = guid {
                analysis.generated.push(guid.to_owned());
            }
        } else if !guid.is_some_and(is_known_guid) {
            analysis.hand_written.push(name);
        }
    }
    analysis
}

fn owns_scheme(settings: &Value, guid: &str) -> bool {
    let list = match settings.get("profiles") {
        Some(Value::Array(list)) => Some(list),
        Some(Value::Object(map)) => map.get("list").and_then(Value::as_array),
        _ => None,
    };
    list.into_iter().flatten().any(|profile| {
        text(profile.get("guid")).is_some_and(|g| g.eq_ignore_ascii_case(guid))
            && profile.get("colorScheme").is_some_and(|v| !v.is_null())
    })
}

pub struct Found {
    pub label: &'static str,
    pub path: PathBuf,
    pub text: String,
}

pub struct Terminal {
    desktop: Arc<dyn Desktop>,
}

impl Terminal {
    pub fn new(desktop: Arc<dyn Desktop>) -> Self {
        Self { desktop }
    }

    fn installed(&self, services: &dyn SystemServices) -> Vec<(&'static str, PathBuf)> {
        let Some(local) = self.desktop.local_app_data() else {
            return Vec::new();
        };
        settings_candidates(&local)
            .into_iter()
            .filter(|(_, path)| services.read_file(path).is_ok_and(|b| b.is_some()))
            .collect()
    }

    fn read(&self, services: &dyn SystemServices) -> Result<Vec<Found>, String> {
        let mut found = Vec::new();
        for (label, path) in self.installed(services) {
            let Some(bytes) = services.read_file(&path).map_err(|e| e.to_string())? else {
                continue;
            };
            let text = String::from_utf8(bytes)
                .map_err(|_| "Windows Terminal's settings.json is not UTF-8 text.".to_owned())?;
            found.push(Found { label, path, text });
        }
        Ok(found)
    }
}

fn push_unique(list: &mut Vec<String>, item: &str) {
    if !list.iter().any(|known| known.eq_ignore_ascii_case(item)) {
        list.push(item.to_owned());
    }
}

impl Integration for Terminal {
    fn id(&self) -> &str {
        ID
    }

    fn name(&self) -> &str {
        "Windows Terminal"
    }

    fn detect(&self, services: &dyn SystemServices) -> Detection {
        let installed = self.installed(services);
        if installed.is_empty() {
            return Detection::not_installed(NOT_INSTALLED);
        }
        let labels: Vec<_> = installed.iter().map(|(label, _)| *label).collect();
        Detection::installed(format!("Found {}", labels.join(", ")))
    }

    fn plan(
        &self,
        request: &ApplyRequest,
        _detection: &Detection,
        services: &dyn SystemServices,
    ) -> Result<Plan, String> {
        let local = self
            .desktop
            .local_app_data()
            .ok_or_else(|| NOT_INSTALLED.to_owned())?;
        let found = self.read(services)?;
        if found.is_empty() {
            return Ok(Plan::skip(NOT_INSTALLED));
        }
        let name = scheme_name(&request.theme.name);
        let mut updates: Vec<String> = Vec::new();
        let mut keeps_own: Vec<String> = Vec::new();
        let mut notes = Vec::new();
        let mut edits = Vec::new();
        for settings in &found {
            let value = jsonc::parse(&settings.text)
                .map_err(|e| syntax_message(&e))?
                .unwrap_or(Value::Null);
            let analysis = analyze(&value);
            for profile in &analysis.keeps_own {
                push_unique(&mut keeps_own, profile);
            }
            if !analysis.defaults_scheme {
                for guid in analysis
                    .generated
                    .iter()
                    .map(String::as_str)
                    .chain([WINDOWS_POWERSHELL, COMMAND_PROMPT])
                {
                    if !owns_scheme(&value, guid) {
                        push_unique(&mut updates, guid);
                    }
                }
            }
            if !analysis.needs_edit() {
                continue;
            }
            if analysis.list_form {
                notes.push(format!(
                    "{} keeps its profiles as a plain list, so its hand-written profiles keep their colors.",
                    settings.label
                ));
                continue;
            }
            let current =
                jsonc::get(&settings.text, DEFAULT_SCHEME).map_err(|e| syntax_message(&e))?;
            if current.as_ref().and_then(Value::as_str) != Some(name.as_str()) {
                edits.push(Operation::SetJsonValue {
                    path: settings.path.clone(),
                    pointer: DEFAULT_SCHEME.to_owned(),
                    value: Some(Value::String(name.clone())),
                });
            }
        }
        for profile in keeps_own {
            notes.push(format!("{profile} keeps its own scheme"));
        }
        let mut fragment = Map::new();
        if !updates.is_empty() {
            fragment.insert(
                "profiles".into(),
                Value::Array(
                    updates
                        .iter()
                        .map(|guid| json!({ "updates": guid, "colorScheme": name }))
                        .collect(),
                ),
            );
        }
        fragment.insert(
            "schemes".into(),
            json!([scheme(&name, &request.theme.palette)]),
        );
        let mut contents = serde_json::to_string_pretty(&Value::Object(fragment))
            .map_err(|e| e.to_string())?
            .into_bytes();
        contents.push(b'\n');
        let path = fragment_path(&local);
        let mut operations = Vec::new();
        if services.read_file(&path).map_err(|e| e.to_string())? != Some(contents.clone()) {
            operations.push(Operation::WriteManagedFile { path, contents });
        }
        operations.extend(edits);
        Ok(Plan::Changes { operations, notes })
    }

    fn reload(&self, services: &dyn SystemServices) -> ReloadResult {
        let installed = self.installed(services);
        if installed.is_empty() {
            return ReloadResult::NotNeeded;
        }
        let now = SystemTime::now();
        let errors: Vec<String> = installed
            .iter()
            .filter_map(|(_, path)| services.set_file_modified(path, now).err())
            .map(|e| e.to_string())
            .collect();
        if errors.is_empty() {
            ReloadResult::Reloaded
        } else {
            ReloadResult::Failed(errors.join("; "))
        }
    }
}

#[cfg(test)]
mod tests {
    use tintcast_engine::Rgba;

    use super::*;

    #[test]
    fn the_scheme_carries_every_terminal_color() {
        let palette = Palette::new(Rgba::rgb(0x10, 0x10, 0x10), Rgba::rgb(0xee, 0xee, 0xee))
            .with_accent(Rgba::rgb(0xd9, 0x4f, 0x70));
        let scheme = scheme("Tintcast Test", &palette);
        let map = scheme.as_object().unwrap();
        assert_eq!(map.len(), 21);
        assert_eq!(map["name"], "Tintcast Test");
        assert_eq!(map["background"], "#101010");
        assert_eq!(map["foreground"], "#eeeeee");
        assert_eq!(map["cursorColor"], "#d94f70");
        assert_eq!(map["selectionBackground"], palette.selection.hex());
        assert_eq!(map["black"], palette.ansi[0].hex());
        assert_eq!(map["purple"], palette.ansi[5].hex());
        assert_eq!(map["brightWhite"], palette.ansi[15].hex());
    }

    #[test]
    fn sorts_profiles_into_generated_hand_written_and_own_schemes() {
        let settings = json!({
            "profiles": {
                "defaults": {},
                "list": [
                    { "guid": WINDOWS_POWERSHELL, "name": "Windows PowerShell" },
                    { "guid": COMMAND_PROMPT, "name": "Command Prompt" },
                    { "guid": "{b453ae62-4e3d-5e58-b989-0a998ec441b8}", "name": "Azure Cloud Shell", "source": "Windows.Terminal.Azure" },
                    { "guid": "{574e775e-4f2a-5b96-ac1e-a2962a402336}", "name": "PowerShell", "source": "Windows.Terminal.PowershellCore", "colorScheme": "One Half Dark" },
                    { "guid": "{11111111-1111-1111-1111-111111111111}", "name": "Hidden", "hidden": true },
                    { "guid": "{22222222-2222-2222-2222-222222222222}", "name": "My SSH box", "commandline": "ssh box" }
                ]
            }
        });
        let analysis = analyze(&settings);
        assert!(!analysis.defaults_scheme);
        assert_eq!(
            analysis.generated,
            ["{b453ae62-4e3d-5e58-b989-0a998ec441b8}"]
        );
        assert_eq!(analysis.keeps_own, ["PowerShell"]);
        assert_eq!(analysis.hand_written, ["My SSH box"]);
        assert!(analysis.needs_edit());
        assert!(owns_scheme(
            &settings,
            "{574E775E-4F2A-5B96-AC1E-A2962A402336}"
        ));
        let mut plain = settings.clone();
        plain["profiles"]["list"].as_array_mut().unwrap().pop();
        assert!(!analyze(&plain).needs_edit());
        plain["profiles"]["defaults"]["colorScheme"] = json!("Campbell");
        assert!(analyze(&plain).needs_edit());
    }

    #[test]
    fn a_syntax_error_names_its_line_and_column() {
        let error = jsonc::parse("{\n  \"profiles\": ]\n}").unwrap_err();
        let message = syntax_message(&error);
        assert!(
            message.starts_with(
                "Windows Terminal's settings.json has a syntax error at line 2, column "
            ),
            "{message}"
        );
        assert!(message.ends_with('.'));
    }
}
