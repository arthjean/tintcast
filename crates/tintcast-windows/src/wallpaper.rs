use std::path::Path;
use std::sync::Arc;

use tintcast_engine::{
    ApplyRequest, Detection, Integration, Operation, Plan, SystemServices, Verification,
};

use crate::desktop::{BackgroundKind, Desktop};
use crate::keys::{BACKGROUND, COLORS};

pub const ID: &str = "wallpaper";
pub const POLICY_SKIP: &str = "A policy on this PC blocks wallpaper changes.";

pub fn stays_off_note(kind: BackgroundKind) -> Option<String> {
    matches!(kind, BackgroundKind::Slideshow | BackgroundKind::Spotlight).then(|| {
        format!(
            "Your {} stays off after Restore; Restore brings back the picture shown now.",
            kind.label()
        )
    })
}

fn same_path(a: &Path, b: &Path) -> bool {
    a.as_os_str()
        .to_string_lossy()
        .eq_ignore_ascii_case(&b.as_os_str().to_string_lossy())
}

pub struct Wallpapers {
    desktop: Arc<dyn Desktop>,
}

impl Wallpapers {
    pub fn new(desktop: Arc<dyn Desktop>) -> Self {
        Self { desktop }
    }
}

impl Integration for Wallpapers {
    fn id(&self) -> &str {
        ID
    }

    fn name(&self) -> &str {
        "Wallpaper"
    }

    fn detect(&self, _services: &dyn SystemServices) -> Detection {
        Detection::installed("Windows desktop")
    }

    fn plan(
        &self,
        request: &ApplyRequest,
        _detection: &Detection,
        services: &dyn SystemServices,
    ) -> Result<Plan, String> {
        if self.desktop.wallpaper_policy_blocks() {
            return Ok(Plan::skip(POLICY_SKIP));
        }
        let theme = &request.theme;
        if theme.wallpapers.is_empty() {
            return Ok(Plan::skip(format!("{} has no wallpaper.", theme.name)));
        }
        let count = theme.wallpapers.len();
        let wallpaper = theme.wallpapers.get(request.wallpaper).ok_or_else(|| {
            format!(
                "{} has {count} wallpapers, so there is no wallpaper {}.",
                theme.name,
                request.wallpaper + 1
            )
        })?;
        let fail = |e: tintcast_engine::SystemError| e.to_string();
        let monitors = services.monitors().map_err(fail)?;
        if monitors.is_empty() {
            return Ok(Plan::skip("Windows reports no connected display."));
        }
        let mut operations = Vec::new();
        if services.wallpaper_position().map_err(fail)? != theme.position {
            operations.push(Operation::SetWallpaperPosition {
                position: theme.position,
            });
        }
        for monitor in monitors {
            let current = services.wallpaper(&monitor).map_err(fail)?;
            if current.is_none_or(|current| !same_path(&current, &wallpaper.image)) {
                operations.push(Operation::SetWallpaper {
                    monitor,
                    image: wallpaper.image.clone(),
                });
            }
        }
        let mut notes = Vec::new();
        let shows_picture = operations
            .iter()
            .any(|op| matches!(op, Operation::SetWallpaper { .. }));
        let kind = if shows_picture {
            self.desktop.background_kind()
        } else {
            BackgroundKind::Picture
        };
        if let Some(note) = stays_off_note(kind) {
            notes.push(note);
        }
        if kind == BackgroundKind::SolidColor
            && let Some(color) = services.read_registry(COLORS, BACKGROUND).map_err(fail)?
        {
            operations.insert(
                0,
                Operation::SetRegistryValue {
                    key: COLORS.to_owned(),
                    name: BACKGROUND.to_owned(),
                    value: Some(color),
                },
            );
            notes.push("Restore brings back your solid color.".to_owned());
        }
        Ok(Plan::Changes { operations, notes })
    }

    fn verify(&self, operations: &[Operation], services: &dyn SystemServices) -> Verification {
        for op in operations {
            let Operation::SetWallpaper { monitor, image } = op else {
                continue;
            };
            match services.wallpaper(monitor) {
                Ok(Some(shown)) if same_path(&shown, image) => {}
                Ok(shown) => {
                    return Verification::Mismatch(format!(
                        "Windows shows {} on display {monitor}",
                        shown.map_or_else(|| "no picture".to_owned(), |p| p.display().to_string())
                    ));
                }
                Err(error) => {
                    return Verification::Mismatch(format!(
                        "Tintcast could not read the wallpaper of display {monitor}: {error}"
                    ));
                }
            }
        }
        Verification::Matches
    }
}
