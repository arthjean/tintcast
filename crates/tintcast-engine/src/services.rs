use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::{Deserialize, Serialize};
use tintcast_theme::WallpaperPosition;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "lowercase")]
pub enum RegistryValue {
    Dword(u32),
    Binary(#[serde(with = "hex_bytes")] Vec<u8>),
    String(String),
}

pub const DISPLAY_NOT_CONNECTED: &str = "Display not connected";

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct SystemError(pub String);

impl SystemError {
    pub fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

pub type SystemResult<T> = Result<T, SystemError>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegistryChange {
    pub key: String,
    pub name: String,
    pub value: Option<RegistryValue>,
}

impl RegistryChange {
    pub fn set(key: &str, name: &str, value: RegistryValue) -> Self {
        Self {
            key: key.to_owned(),
            name: name.to_owned(),
            value: Some(value),
        }
    }

    pub fn delete(key: &str, name: &str) -> Self {
        Self {
            key: key.to_owned(),
            name: name.to_owned(),
            value: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{error}")]
pub struct CommitError {
    pub index: Option<usize>,
    pub error: SystemError,
}

impl CommitError {
    pub fn at(index: usize, error: SystemError) -> Self {
        Self {
            index: Some(index),
            error,
        }
    }
}

pub trait SystemServices: Send + Sync {
    fn read_registry(&self, key: &str, name: &str) -> SystemResult<Option<RegistryValue>>;
    fn commit_registry(&self, changes: &[RegistryChange]) -> Result<(), CommitError>;

    fn monitors(&self) -> SystemResult<Vec<String>>;
    fn wallpaper(&self, monitor: &str) -> SystemResult<Option<PathBuf>>;
    fn set_wallpaper(&self, monitor: &str, image: Option<&Path>) -> SystemResult<()>;
    fn wallpaper_position(&self) -> SystemResult<WallpaperPosition>;
    fn set_wallpaper_position(&self, position: WallpaperPosition) -> SystemResult<()>;

    fn read_file(&self, path: &Path) -> SystemResult<Option<Vec<u8>>>;
    fn write_file(&self, path: &Path, bytes: &[u8]) -> SystemResult<()>;
    fn delete_file(&self, path: &Path) -> SystemResult<()>;
    fn set_file_modified(&self, path: &Path, time: SystemTime) -> SystemResult<()>;

    fn broadcast_color_change(&self) -> SystemResult<()>;
}

mod hex_bytes {
    use std::fmt::Write;

    use serde::{Deserialize, Deserializer, Serializer, de::Error};

    pub fn serialize<S: Serializer>(bytes: &[u8], serializer: S) -> Result<S::Ok, S::Error> {
        let mut text = String::with_capacity(bytes.len() * 2);
        for byte in bytes {
            let _ = write!(text, "{byte:02x}");
        }
        serializer.serialize_str(&text)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<u8>, D::Error> {
        let text = String::deserialize(deserializer)?;
        if text.len() % 2 != 0 {
            return Err(D::Error::custom("hex text has an odd length"));
        }
        (0..text.len())
            .step_by(2)
            .map(|i| {
                text.get(i..i + 2)
                    .and_then(|pair| u8::from_str_radix(pair, 16).ok())
                    .ok_or_else(|| D::Error::custom("hex text has a character that is not hex"))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binary_values_round_trip_as_hex() {
        let value = RegistryValue::Binary(vec![0x00, 0x7f, 0xff, 0x10]);
        let text = serde_json::to_string(&value).unwrap();
        assert_eq!(text, r#"{"type":"binary","data":"007fff10"}"#);
        assert_eq!(serde_json::from_str::<RegistryValue>(&text).unwrap(), value);
    }

    #[test]
    fn string_values_round_trip() {
        let value = RegistryValue::String("30 30 30".into());
        let text = serde_json::to_string(&value).unwrap();
        assert_eq!(text, r#"{"type":"string","data":"30 30 30"}"#);
        assert_eq!(serde_json::from_str::<RegistryValue>(&text).unwrap(), value);
    }

    #[test]
    fn malformed_hex_is_rejected() {
        assert!(serde_json::from_str::<RegistryValue>(r#"{"type":"binary","data":"0g"}"#).is_err());
        assert!(
            serde_json::from_str::<RegistryValue>(r#"{"type":"binary","data":"abc"}"#).is_err()
        );
    }
}
