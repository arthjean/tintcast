mod atomic;
mod data_dir;
mod engine;
mod error;
pub mod fake;
mod integration;
pub mod jsonc;
mod library;
mod lock;
mod operation;
mod services;
mod store;

pub use atomic::{StagedWrite, stage, write_atomic};
pub use data_dir::{DataDir, DataDirError};
pub use engine::{
    ApplyReport, Engine, IntegrationResult, InterruptedRun, Outcome, PlannedIntegration,
    PlannedOutcome, RestoreItem, RestoreReport, RestoreResult, RollbackReport, Startup, UndoReport,
};
pub use error::EngineError;
pub use integration::{ApplyRequest, Detection, Integration, Plan, ReloadResult, Verification};
pub use library::{
    ALREADY_IN_LIBRARY, BUNDLED_REFUSED, ImportItem, ImportOutcome, ImportReport, Library,
    LibraryError, LibraryOptions, LoadReport,
};
pub use lock::{EngineLock, LockGuard};
pub use operation::{Operation, ResourceState, sha256_hex};
pub use services::{RegistryValue, SystemError, SystemResult, SystemServices};
pub use store::{
    AppliedValue, HistoryEntry, JournalRun, JournalStep, LedgerEntry, StepState, VERSION,
};
pub use tintcast_theme::{Mode, Origin, Palette, Rgba, Theme, Wallpaper, WallpaperPosition};
