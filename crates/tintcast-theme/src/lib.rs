use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Theme {
    pub id: String,
    pub name: String,
}

impl Theme {
    pub fn new(id: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WallpaperPosition {
    #[default]
    Fill,
    Fit,
    Stretch,
    Center,
    Tile,
    Span,
}

impl WallpaperPosition {
    pub const ALL: [WallpaperPosition; 6] = [
        Self::Fill,
        Self::Fit,
        Self::Stretch,
        Self::Center,
        Self::Tile,
        Self::Span,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Fill => "Fill",
            Self::Fit => "Fit",
            Self::Stretch => "Stretch",
            Self::Center => "Center",
            Self::Tile => "Tile",
            Self::Span => "Span",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fill_is_the_default_position() {
        assert_eq!(WallpaperPosition::default(), WallpaperPosition::Fill);
    }

    #[test]
    fn positions_have_distinct_labels() {
        let mut labels: Vec<_> = WallpaperPosition::ALL.iter().map(|p| p.label()).collect();
        labels.sort_unstable();
        labels.dedup();
        assert_eq!(labels.len(), 6);
    }
}
