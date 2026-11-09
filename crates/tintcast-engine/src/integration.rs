use tintcast_theme::Theme;

use crate::operation::Operation;
use crate::services::SystemServices;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Detection {
    pub installed: bool,
    pub reason: String,
}

impl Detection {
    pub fn installed(reason: impl Into<String>) -> Self {
        Self {
            installed: true,
            reason: reason.into(),
        }
    }

    pub fn not_installed(reason: impl Into<String>) -> Self {
        Self {
            installed: false,
            reason: reason.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Plan {
    Changes {
        operations: Vec<Operation>,
        notes: Vec<String>,
    },
    Skip {
        reason: String,
    },
}

impl Plan {
    pub fn changes(operations: Vec<Operation>) -> Self {
        Self::Changes {
            operations,
            notes: Vec::new(),
        }
    }

    pub fn skip(reason: impl Into<String>) -> Self {
        Self::Skip {
            reason: reason.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReloadResult {
    Reloaded,
    NotNeeded,
    Failed(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verification {
    Matches,
    Mismatch(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplyRequest {
    pub theme: Theme,
    pub wallpaper: usize,
    pub only: Option<Vec<String>>,
}

impl ApplyRequest {
    pub fn new(theme: Theme) -> Self {
        Self {
            theme,
            wallpaper: 0,
            only: None,
        }
    }

    pub fn with_wallpaper(mut self, index: usize) -> Self {
        self.wallpaper = index;
        self
    }

    pub fn only(mut self, integrations: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.only = Some(integrations.into_iter().map(Into::into).collect());
        self
    }

    pub fn includes(&self, integration: &str) -> bool {
        self.only
            .as_ref()
            .is_none_or(|only| only.iter().any(|id| id == integration))
    }
}

pub trait Integration: Send + Sync {
    fn id(&self) -> &str;

    fn name(&self) -> &str;

    fn detect(&self, services: &dyn SystemServices) -> Detection;

    fn plan(
        &self,
        request: &ApplyRequest,
        detection: &Detection,
        services: &dyn SystemServices,
    ) -> Result<Plan, String>;

    fn reload(&self, services: &dyn SystemServices) -> ReloadResult {
        let _ = services;
        ReloadResult::NotNeeded
    }

    fn verify(&self, operations: &[Operation], services: &dyn SystemServices) -> Verification {
        let _ = (operations, services);
        Verification::Matches
    }
}
