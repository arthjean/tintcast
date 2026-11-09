use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use jiff::Timestamp;
use tintcast_theme::Theme;

use crate::data_dir::DataDir;
use crate::error::EngineError;
use crate::integration::{ApplyRequest, Detection, Integration, Plan, ReloadResult, Verification};
use crate::jsonc;
use crate::lock::EngineLock;
use crate::operation::{Operation, ResourceState, sha256_hex};
use crate::services::SystemServices;
use crate::store::{
    AppliedValue, HistoryEntry, JournalRun, JournalStep, Ledger, LedgerEntry, StepState, Store,
    VERSION,
};

static RUN_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, PartialEq)]
pub enum PlannedOutcome {
    Changes {
        operations: Vec<Operation>,
        notes: Vec<String>,
    },
    Skip(String),
    Error(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct PlannedIntegration {
    pub id: String,
    pub name: String,
    pub detection: Detection,
    pub outcome: PlannedOutcome,
}

impl PlannedIntegration {
    pub fn summaries(&self) -> Vec<String> {
        match &self.outcome {
            PlannedOutcome::Changes { operations, .. } => {
                operations.iter().map(Operation::summary).collect()
            }
            _ => Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Applied,
    Unverified(String),
    Skipped(String),
    Failed {
        operation: Option<String>,
        error: String,
        rollback_errors: Vec<String>,
    },
}

impl Outcome {
    pub fn changed(&self) -> bool {
        matches!(self, Self::Applied | Self::Unverified(_))
    }

    pub fn is_failure(&self) -> bool {
        matches!(self, Self::Failed { .. })
    }

    pub fn message(&self) -> String {
        match self {
            Self::Applied => "Applied".to_owned(),
            Self::Unverified(reason) => format!("Applied, but {reason}"),
            Self::Skipped(reason) => reason.clone(),
            Self::Failed {
                operation,
                error,
                rollback_errors,
            } => {
                let error = error.trim_end_matches('.');
                let cause = match operation {
                    Some(summary) => format!("{summary}: {error}"),
                    None => error.to_owned(),
                };
                if rollback_errors.is_empty() {
                    format!("{cause}. Tintcast rolled back this part.")
                } else {
                    format!(
                        "{cause}. Tintcast could not roll back: {}",
                        rollback_errors.join("; ")
                    )
                }
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntegrationResult {
    pub id: String,
    pub name: String,
    pub outcome: Outcome,
    pub operations: Vec<String>,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplyReport {
    pub run_id: String,
    pub theme_id: String,
    pub theme_name: String,
    pub results: Vec<IntegrationResult>,
}

impl ApplyReport {
    pub fn failed(&self) -> bool {
        self.results.iter().any(|r| r.outcome.is_failure())
    }

    pub fn result(&self, integration: &str) -> Option<&IntegrationResult> {
        self.results.iter().find(|r| r.id == integration)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RestoreResult {
    Restored,
    Unchanged,
    KeptYourChange,
    Failed(String),
}

impl RestoreResult {
    pub fn message(&self) -> String {
        match self {
            Self::Restored => "Restored".to_owned(),
            Self::Unchanged => "Unchanged".to_owned(),
            Self::KeptYourChange => "Kept your change".to_owned(),
            Self::Failed(error) => format!("Could not restore: {error}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestoreItem {
    pub resource: String,
    pub integration: String,
    pub summary: String,
    pub result: RestoreResult,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RestoreReport {
    pub items: Vec<RestoreItem>,
    pub reloads: Vec<(String, ReloadResult)>,
}

impl RestoreReport {
    pub fn item(&self, resource: &str) -> Option<&RestoreItem> {
        self.items.iter().find(|i| i.resource == resource)
    }

    pub fn failed(&self) -> bool {
        self.items
            .iter()
            .any(|i| matches!(i.result, RestoreResult::Failed(_)))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UndoReport {
    NothingToUndo,
    Reapplied(ApplyReport),
    Restored(RestoreReport),
}

impl UndoReport {
    pub const NOTHING_TO_UNDO: &'static str = "Nothing to undo";
}

#[derive(Debug, Clone, PartialEq)]
pub struct InterruptedRun {
    pub run_id: String,
    pub theme_id: String,
    pub theme_name: String,
    pub started: Timestamp,
    pub applied: Vec<JournalStep>,
    pub pending: Vec<JournalStep>,
}

impl InterruptedRun {
    fn from_run(run: JournalRun) -> Self {
        let pick = |state: StepState| {
            run.steps
                .iter()
                .filter(|s| s.state == state)
                .cloned()
                .collect()
        };
        Self {
            applied: pick(StepState::Applied),
            pending: pick(StepState::Pending),
            run_id: run.run_id,
            theme_id: run.theme_id,
            theme_name: run.theme_name,
            started: run.started,
        }
    }

    pub fn message(&self) -> String {
        format!(
            "Tintcast was interrupted while applying {}.",
            self.theme_name
        )
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Startup {
    pub interrupted: Vec<InterruptedRun>,
    pub corrupt_journals: Vec<PathBuf>,
    pub ledger_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RollbackReport {
    pub run_id: String,
    pub items: Vec<RestoreItem>,
}

struct Done {
    op: Operation,
    applied: ResourceState,
    before: ResourceState,
    before_bytes: Option<Option<Vec<u8>>>,
    first_touch: bool,
    step: usize,
}

type StepFailure = (String, String);

pub struct Engine {
    store: Store,
    services: Arc<dyn SystemServices>,
    integrations: Vec<Box<dyn Integration>>,
    lock: EngineLock,
}

fn is_hollow(value: &serde_json::Value) -> bool {
    value
        .as_object()
        .is_some_and(|map| map.values().all(is_hollow))
}

fn new_run_id() -> String {
    format!(
        "{}-{}-{}",
        Timestamp::now().as_millisecond(),
        std::process::id(),
        RUN_COUNTER.fetch_add(1, Ordering::Relaxed)
    )
}

impl Engine {
    pub fn new(
        data: DataDir,
        services: Arc<dyn SystemServices>,
        integrations: Vec<Box<dyn Integration>>,
    ) -> Self {
        Self {
            store: Store::new(data),
            services,
            integrations,
            lock: EngineLock::default(),
        }
    }

    pub fn open(
        data: DataDir,
        services: Arc<dyn SystemServices>,
        integrations: Vec<Box<dyn Integration>>,
    ) -> Result<(Self, Startup), EngineError> {
        let engine = Self::new(data, services, integrations);
        let startup = engine.startup()?;
        Ok((engine, startup))
    }

    pub fn with_lock(mut self, lock: EngineLock) -> Self {
        self.lock = lock;
        self
    }

    pub fn data_dir(&self) -> &DataDir {
        self.store.dir()
    }

    pub fn services(&self) -> &dyn SystemServices {
        self.services.as_ref()
    }

    pub fn startup(&self) -> Result<Startup, EngineError> {
        let _guard = self.lock.acquire()?;
        let scan = self.store.scan_journals()?;
        Ok(Startup {
            interrupted: scan
                .runs
                .into_iter()
                .map(InterruptedRun::from_run)
                .collect(),
            corrupt_journals: scan.corrupt.into_iter().map(|(_, path)| path).collect(),
            ledger_error: self.store.load_ledger().err().map(|e| e.to_string()),
        })
    }

    pub fn ledger_entries(&self) -> Result<Vec<LedgerEntry>, EngineError> {
        Ok(self.store.load_ledger()?.oldest_first())
    }

    pub fn history(&self) -> Result<Vec<HistoryEntry>, EngineError> {
        self.store.load_history()
    }

    fn integration(&self, id: &str) -> Option<&dyn Integration> {
        self.integrations
            .iter()
            .find(|i| i.id() == id)
            .map(|i| i.as_ref())
    }

    pub fn plan(&self, request: &ApplyRequest) -> Vec<PlannedIntegration> {
        let services = self.services.as_ref();
        self.integrations
            .iter()
            .filter(|i| request.includes(i.id()))
            .map(|integration| {
                let detection = integration.detect(services);
                let outcome = if !detection.installed {
                    PlannedOutcome::Skip(detection.reason.clone())
                } else {
                    match integration.plan(request, &detection, services) {
                        Ok(Plan::Changes { operations, notes }) => {
                            PlannedOutcome::Changes { operations, notes }
                        }
                        Ok(Plan::Skip { reason }) => PlannedOutcome::Skip(reason),
                        Err(error) => PlannedOutcome::Error(error),
                    }
                };
                PlannedIntegration {
                    id: integration.id().to_owned(),
                    name: integration.name().to_owned(),
                    detection,
                    outcome,
                }
            })
            .collect()
    }

    pub fn apply(&self, request: &ApplyRequest) -> Result<ApplyReport, EngineError> {
        let _guard = self.lock.acquire()?;
        self.apply_locked(request, 0)
    }

    fn apply_locked(
        &self,
        request: &ApplyRequest,
        replace_history: usize,
    ) -> Result<ApplyReport, EngineError> {
        let mut ledger = self.store.load_ledger()?;
        let planned = self.plan(request);
        let mut run = JournalRun {
            version: VERSION,
            run_id: new_run_id(),
            theme_id: request.theme.id.clone(),
            theme_name: request.theme.name.clone(),
            started: Timestamp::now(),
            steps: Vec::new(),
        };
        self.store.save_journal(&run)?;
        let mut results = Vec::new();
        let mut applied = Vec::new();
        let mut changed = Vec::new();
        for item in planned {
            let result = match (&item.outcome, self.integration(&item.id)) {
                (PlannedOutcome::Changes { operations, notes }, Some(integration))
                    if !operations.is_empty() =>
                {
                    self.apply_integration(
                        &mut ledger,
                        &mut run,
                        integration,
                        operations,
                        notes,
                        &mut applied,
                    )
                }
                (PlannedOutcome::Changes { notes, .. }, _) => IntegrationResult {
                    id: item.id.clone(),
                    name: item.name.clone(),
                    outcome: Outcome::Skipped("Nothing to change".to_owned()),
                    operations: Vec::new(),
                    notes: notes.clone(),
                },
                (PlannedOutcome::Skip(reason), _) => IntegrationResult {
                    id: item.id.clone(),
                    name: item.name.clone(),
                    outcome: Outcome::Skipped(reason.clone()),
                    operations: Vec::new(),
                    notes: Vec::new(),
                },
                (PlannedOutcome::Error(error), _) => IntegrationResult {
                    id: item.id.clone(),
                    name: item.name.clone(),
                    outcome: Outcome::Failed {
                        operation: None,
                        error: error.clone(),
                        rollback_errors: Vec::new(),
                    },
                    operations: Vec::new(),
                    notes: Vec::new(),
                },
            };
            if result.outcome.changed() {
                changed.push(result.id.clone());
            }
            results.push(result);
        }
        if !changed.is_empty() {
            let mut history = self.store.load_history()?;
            history.truncate(history.len().saturating_sub(replace_history));
            history.push(HistoryEntry {
                theme_id: request.theme.id.clone(),
                theme_name: request.theme.name.clone(),
                wallpaper: request.wallpaper,
                at: Timestamp::now(),
                integrations: changed,
                applied,
            });
            self.store.save_history(&history)?;
        }
        self.store.remove_journal(&run.run_id)?;
        Ok(ApplyReport {
            run_id: run.run_id,
            theme_id: request.theme.id.clone(),
            theme_name: request.theme.name.clone(),
            results,
        })
    }

    fn apply_integration(
        &self,
        ledger: &mut Ledger,
        run: &mut JournalRun,
        integration: &dyn Integration,
        operations: &[Operation],
        notes: &[String],
        applied: &mut Vec<AppliedValue>,
    ) -> IntegrationResult {
        let mut done: Vec<Done> = Vec::new();
        let mut failure = None;
        for op in operations {
            match self.apply_one(ledger, run, integration.id(), op) {
                Ok(step) => done.push(step),
                Err(error) => {
                    failure = Some(error);
                    break;
                }
            }
        }
        let summaries = operations.iter().map(Operation::summary).collect();
        let mut notes = notes.to_vec();
        if let Some((operation, error)) = failure {
            let rollback_errors = self.roll_back_done(ledger, run, &done);
            return IntegrationResult {
                id: integration.id().to_owned(),
                name: integration.name().to_owned(),
                outcome: Outcome::Failed {
                    operation: Some(operation),
                    error,
                    rollback_errors,
                },
                operations: summaries,
                notes,
            };
        }
        applied.extend(done.iter().map(|d| AppliedValue {
            resource: d.op.resource(),
            value: d.applied.clone(),
        }));
        if let ReloadResult::Failed(error) = integration.reload(self.services.as_ref()) {
            notes.push(format!("Reload failed: {error}"));
        }
        let outcome = match self.verify_done(&done) {
            Verification::Mismatch(reason) => Outcome::Unverified(reason),
            Verification::Matches => match integration.verify(operations, self.services.as_ref()) {
                Verification::Matches => Outcome::Applied,
                Verification::Mismatch(reason) => Outcome::Unverified(reason),
            },
        };
        IntegrationResult {
            id: integration.id().to_owned(),
            name: integration.name().to_owned(),
            outcome,
            operations: summaries,
            notes,
        }
    }

    fn apply_one(
        &self,
        ledger: &mut Ledger,
        run: &mut JournalRun,
        integration: &str,
        op: &Operation,
    ) -> Result<Done, StepFailure> {
        let summary = op.summary();
        let fail = |error: String| (summary.clone(), error);
        let resource = op.resource();
        let before = self.state_of(op).map_err(fail)?;
        let before_bytes = match op.file_path() {
            Some(path) => Some(
                self.services
                    .read_file(path)
                    .map_err(|e| fail(e.to_string()))?,
            ),
            None => None,
        };
        let file_backup = match &before_bytes {
            Some(Some(bytes)) => Some(self.store.backup(bytes).map_err(|e| fail(e.to_string()))?),
            _ => None,
        };
        let first_touch = !ledger.contains(&resource);
        let previous = ledger.get(&resource).cloned();
        let put_back = |ledger: &mut Ledger| match &previous {
            Some(previous) => ledger.insert(previous.clone()),
            None => {
                ledger.remove(&resource);
            }
        };
        let target = op.target_state();
        match ledger.get_mut(&resource) {
            Some(entry) => entry.pending = Some(target),
            None => {
                let now = Timestamp::now();
                ledger.insert(LedgerEntry {
                    sequence: ledger.next_sequence(),
                    resource: resource.clone(),
                    integration: integration.to_owned(),
                    operation: op.clone(),
                    original: before.clone(),
                    applied: before.clone(),
                    pending: Some(target),
                    file_backup: file_backup.clone(),
                    first_touched: now,
                    last_applied: now,
                    error: None,
                });
            }
        }
        if let Err(error) = self.store.save_ledger(ledger) {
            put_back(ledger);
            return Err(fail(error.to_string()));
        }
        let abandon = |ledger: &mut Ledger| {
            put_back(ledger);
            let _ = self.store.save_ledger(ledger);
        };
        run.steps.push(JournalStep {
            integration: integration.to_owned(),
            resource: resource.clone(),
            summary: summary.clone(),
            operation: op.clone(),
            before: before.clone(),
            file_backup,
            state: StepState::Pending,
        });
        let step = run.steps.len() - 1;
        if let Err(error) = self.store.save_journal(run) {
            run.steps[step].state = StepState::Failed;
            abandon(ledger);
            return Err(fail(error.to_string()));
        }
        let applied = match self.execute(op) {
            Ok(applied) => applied,
            Err(error) => {
                run.steps[step].state = StepState::Failed;
                let _ = self.store.save_journal(run);
                abandon(ledger);
                return Err(fail(error));
            }
        };
        if let Some(entry) = ledger.get_mut(&resource) {
            entry.applied = applied.clone();
            entry.pending = None;
            entry.operation = op.clone();
            entry.integration = integration.to_owned();
            entry.last_applied = Timestamp::now();
            entry.error = None;
        }
        if let Err(error) = self.store.save_ledger(ledger) {
            let _ = self.revert(op, &before, before_bytes.as_ref());
            abandon(ledger);
            run.steps[step].state = StepState::Failed;
            let _ = self.store.save_journal(run);
            return Err(fail(error.to_string()));
        }
        run.steps[step].state = StepState::Applied;
        let _ = self.store.save_journal(run);
        Ok(Done {
            op: op.clone(),
            applied,
            before,
            before_bytes,
            first_touch,
            step,
        })
    }

    fn roll_back_done(
        &self,
        ledger: &mut Ledger,
        run: &mut JournalRun,
        done: &[Done],
    ) -> Vec<String> {
        let mut errors = Vec::new();
        for item in done.iter().rev() {
            match self.revert(&item.op, &item.before, item.before_bytes.as_ref()) {
                Ok(()) => {
                    let resource = item.op.resource();
                    if item.first_touch {
                        ledger.remove(&resource);
                    } else if let Some(entry) = ledger.get_mut(&resource) {
                        entry.applied = item.before.clone();
                    }
                    run.steps[item.step].state = StepState::RolledBack;
                }
                Err(error) => errors.push(format!("{}: {error}", item.op.summary())),
            }
        }
        if let Err(error) = self.store.save_ledger(ledger) {
            errors.push(error.to_string());
        }
        let _ = self.store.save_journal(run);
        errors
    }

    fn verify_done(&self, done: &[Done]) -> Verification {
        for item in done {
            match self.state_of(&item.op) {
                Ok(state) if state == item.applied => {}
                Ok(_) => {
                    return Verification::Mismatch(format!(
                        "Windows reports a different value after: {}",
                        item.op.summary()
                    ));
                }
                Err(error) => {
                    return Verification::Mismatch(format!(
                        "Tintcast could not read back {}: {error}",
                        item.op.resource()
                    ));
                }
            }
        }
        Verification::Matches
    }

    pub fn restore(&self) -> Result<RestoreReport, EngineError> {
        let _guard = self.lock.acquire()?;
        self.restore_locked()
    }

    fn restore_locked(&self) -> Result<RestoreReport, EngineError> {
        let mut ledger = self.store.load_ledger()?;
        let mut report = RestoreReport::default();
        let mut touched: Vec<String> = Vec::new();
        for entry in ledger.newest_first() {
            if !touched.contains(&entry.integration) {
                touched.push(entry.integration.clone());
            }
            let result = match self.state_of(&entry.operation) {
                Err(error) => RestoreResult::Failed(error),
                Ok(current) if current == entry.original => RestoreResult::Unchanged,
                Ok(current)
                    if current == entry.applied || entry.pending.as_ref() == Some(&current) =>
                {
                    match self
                        .revert(&entry.operation, &entry.original, None)
                        .and_then(|()| {
                            self.tidy_json(
                                &entry.operation,
                                &entry.original,
                                entry.file_backup.as_deref(),
                            )
                        }) {
                        Ok(()) => RestoreResult::Restored,
                        Err(error) => RestoreResult::Failed(error),
                    }
                }
                Ok(_) => RestoreResult::KeptYourChange,
            };
            match &result {
                RestoreResult::Failed(error) => {
                    if let Some(kept) = ledger.get_mut(&entry.resource) {
                        kept.error = Some(error.clone());
                    }
                }
                _ => {
                    ledger.remove(&entry.resource);
                }
            }
            report.items.push(RestoreItem {
                resource: entry.resource.clone(),
                integration: entry.integration.clone(),
                summary: entry.operation.summary(),
                result,
            });
        }
        self.store.save_ledger(&ledger)?;
        self.store.save_history(&[])?;
        for id in touched {
            if let Some(integration) = self.integration(&id) {
                report
                    .reloads
                    .push((id, integration.reload(self.services.as_ref())));
            }
        }
        Ok(report)
    }

    pub fn undo(&self, library: &dyn Fn(&str) -> Option<Theme>) -> Result<UndoReport, EngineError> {
        let _guard = self.lock.acquire()?;
        let history = self.store.load_history()?;
        match history.len() {
            0 => Ok(UndoReport::NothingToUndo),
            1 => Ok(UndoReport::Restored(self.restore_locked()?)),
            count => {
                let previous = &history[count - 2];
                let theme = library(&previous.theme_id)
                    .ok_or_else(|| EngineError::ThemeMissing(previous.theme_name.clone()))?;
                let request = ApplyRequest {
                    theme,
                    wallpaper: previous.wallpaper,
                    only: Some(previous.integrations.clone()),
                };
                Ok(UndoReport::Reapplied(self.apply_locked(&request, 2)?))
            }
        }
    }

    pub fn roll_back(&self, run_id: &str) -> Result<RollbackReport, EngineError> {
        let _guard = self.lock.acquire()?;
        let run = self.store.load_journal(run_id)?;
        let mut ledger = self.store.load_ledger()?;
        let mut items = Vec::new();
        for step in run
            .steps
            .iter()
            .rev()
            .filter(|s| matches!(s.state, StepState::Applied | StepState::Pending))
        {
            let result = match self.state_of(&step.operation) {
                Err(error) => RestoreResult::Failed(error),
                Ok(current) if current == step.before => RestoreResult::Unchanged,
                Ok(_) => match self
                    .revert(&step.operation, &step.before, None)
                    .and_then(|()| {
                        self.tidy_json(&step.operation, &step.before, step.file_backup.as_deref())
                    }) {
                    Ok(()) => RestoreResult::Restored,
                    Err(error) => RestoreResult::Failed(error),
                },
            };
            if !matches!(result, RestoreResult::Failed(_)) {
                let original = ledger.get(&step.resource).map(|e| e.original.clone());
                match original {
                    Some(original) if original == step.before => {
                        ledger.remove(&step.resource);
                    }
                    Some(_) => {
                        if let Some(entry) = ledger.get_mut(&step.resource) {
                            entry.applied = step.before.clone();
                            entry.pending = None;
                        }
                    }
                    None => {}
                }
            }
            items.push(RestoreItem {
                resource: step.resource.clone(),
                integration: step.integration.clone(),
                summary: step.summary.clone(),
                result,
            });
        }
        self.store.save_ledger(&ledger)?;
        if !items
            .iter()
            .any(|i| matches!(i.result, RestoreResult::Failed(_)))
        {
            self.store.remove_journal(run_id)?;
        }
        Ok(RollbackReport {
            run_id: run_id.to_owned(),
            items,
        })
    }

    pub fn keep_interrupted(&self, run_id: &str) -> Result<(), EngineError> {
        let _guard = self.lock.acquire()?;
        let run = self.store.load_journal(run_id)?;
        let mut ledger = self.store.load_ledger()?;
        for step in run.steps.iter().filter(|s| s.state == StepState::Pending) {
            let Some(entry) = ledger.get_mut(&step.resource) else {
                continue;
            };
            let Some(pending) = entry.pending.clone() else {
                continue;
            };
            if let Ok(current) = self.state_of(&step.operation) {
                if current == pending {
                    entry.applied = pending;
                }
                entry.pending = None;
            }
        }
        self.store.save_ledger(&ledger)?;
        self.store.remove_journal(run_id)
    }

    fn state_of(&self, op: &Operation) -> Result<ResourceState, String> {
        let services = self.services.as_ref();
        match op {
            Operation::SetRegistryValue { key, name, .. } => services
                .read_registry(key, name)
                .map(|value| {
                    value.map_or(ResourceState::Absent, |value| ResourceState::Registry {
                        value,
                    })
                })
                .map_err(|e| e.to_string()),
            Operation::SetWallpaper { monitor, .. } => services
                .wallpaper(monitor)
                .map(|image| match image {
                    Some(image) if !image.as_os_str().is_empty() => {
                        ResourceState::Wallpaper { image }
                    }
                    _ => ResourceState::Absent,
                })
                .map_err(|e| e.to_string()),
            Operation::SetWallpaperPosition { .. } => services
                .wallpaper_position()
                .map(|position| ResourceState::WallpaperPosition { position })
                .map_err(|e| e.to_string()),
            Operation::WriteManagedFile { path, .. } => services
                .read_file(path)
                .map(|bytes| match bytes {
                    Some(bytes) => ResourceState::File {
                        sha256: sha256_hex(&bytes),
                    },
                    None => ResourceState::Absent,
                })
                .map_err(|e| e.to_string()),
            Operation::SetJsonValue { path, pointer, .. } => {
                let Some(text) = self.read_text(path)? else {
                    return Ok(ResourceState::Absent);
                };
                jsonc::get(&text, pointer)
                    .map(|value| {
                        value.map_or(ResourceState::Absent, |value| ResourceState::Json { value })
                    })
                    .map_err(|e| format!("{} has a {e}", path.display()))
            }
        }
    }

    fn read_text(&self, path: &std::path::Path) -> Result<Option<String>, String> {
        let Some(bytes) = self.services.read_file(path).map_err(|e| e.to_string())? else {
            return Ok(None);
        };
        String::from_utf8(bytes)
            .map(Some)
            .map_err(|_| format!("{} is not UTF-8 text", path.display()))
    }

    fn edit_json(
        &self,
        path: &std::path::Path,
        pointer: &str,
        value: Option<&serde_json::Value>,
        create: bool,
    ) -> Result<(), String> {
        let text = match self.read_text(path)? {
            Some(text) => text,
            None if create => String::new(),
            None => return Ok(()),
        };
        let edited = jsonc::set(&text, pointer, value)
            .map_err(|e| format!("{} has a {e}", path.display()))?;
        if edited != text {
            self.services
                .write_file(path, edited.as_bytes())
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    fn tidy_json(
        &self,
        op: &Operation,
        target: &ResourceState,
        file_backup: Option<&str>,
    ) -> Result<(), String> {
        let Operation::SetJsonValue { path, pointer, .. } = op else {
            return Ok(());
        };
        if *target != ResourceState::Absent {
            return Ok(());
        }
        let Some(text) = self.read_text(path)? else {
            return Ok(());
        };
        let Some(sha) = file_backup else {
            let hollow = match jsonc::parse(&text) {
                Ok(None) => true,
                Ok(Some(value)) => is_hollow(&value),
                Err(_) => false,
            };
            if hollow {
                self.services.delete_file(path).map_err(|e| e.to_string())?;
            }
            return Ok(());
        };
        let Ok(original) = self.store.read_backup(sha) else {
            return Ok(());
        };
        let original = String::from_utf8_lossy(&original);
        let mut tidied = text.clone();
        for ancestor in jsonc::ancestors(pointer) {
            let created = matches!(jsonc::get(&original, ancestor), Ok(None));
            let empty = matches!(
                jsonc::get(&tidied, ancestor),
                Ok(Some(serde_json::Value::Object(map))) if map.is_empty()
            );
            if !(created && empty) {
                break;
            }
            tidied = jsonc::set(&tidied, ancestor, None)
                .map_err(|e| format!("{} has a {e}", path.display()))?;
        }
        if tidied != text {
            self.services
                .write_file(path, tidied.as_bytes())
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    fn execute(&self, op: &Operation) -> Result<ResourceState, String> {
        let services = self.services.as_ref();
        match op {
            Operation::SetRegistryValue {
                key,
                name,
                value: Some(value),
            } => services.write_registry(key, name, value),
            Operation::SetRegistryValue {
                key,
                name,
                value: None,
            } => services.delete_registry(key, name),
            Operation::SetWallpaper { monitor, image } => {
                services.set_wallpaper(monitor, Some(image))
            }
            Operation::SetWallpaperPosition { position } => {
                services.set_wallpaper_position(*position)
            }
            Operation::WriteManagedFile { path, contents } => services.write_file(path, contents),
            Operation::SetJsonValue {
                path,
                pointer,
                value,
            } => {
                return self
                    .edit_json(path, pointer, value.as_ref(), true)
                    .map(|()| op.target_state());
            }
        }
        .map(|()| op.target_state())
        .map_err(|e| e.to_string())
    }

    fn revert(
        &self,
        op: &Operation,
        target: &ResourceState,
        before_bytes: Option<&Option<Vec<u8>>>,
    ) -> Result<(), String> {
        let services = self.services.as_ref();
        let system = |result: crate::services::SystemResult<()>| result.map_err(|e| e.to_string());
        if let (Some(path), Some(bytes)) = (op.file_path(), before_bytes) {
            return system(match bytes {
                Some(bytes) => services.write_file(path, bytes),
                None => services.delete_file(path),
            });
        }
        match (op, target) {
            (Operation::SetRegistryValue { key, name, .. }, ResourceState::Registry { value }) => {
                system(services.write_registry(key, name, value))
            }
            (Operation::SetRegistryValue { key, name, .. }, ResourceState::Absent) => {
                system(services.delete_registry(key, name))
            }
            (Operation::SetWallpaper { monitor, .. }, ResourceState::Wallpaper { image }) => {
                system(services.set_wallpaper(monitor, Some(image)))
            }
            (Operation::SetWallpaper { monitor, .. }, ResourceState::Absent) => {
                system(services.set_wallpaper(monitor, None))
            }
            (
                Operation::SetWallpaperPosition { .. },
                ResourceState::WallpaperPosition { position },
            ) => system(services.set_wallpaper_position(*position)),
            (Operation::WriteManagedFile { path, .. }, ResourceState::Absent) => {
                system(services.delete_file(path))
            }
            (Operation::WriteManagedFile { path, .. }, ResourceState::File { sha256 }) => {
                let bytes = self.store.read_backup(sha256)?;
                system(services.write_file(path, &bytes))
            }
            (Operation::SetJsonValue { path, pointer, .. }, ResourceState::Json { value }) => {
                self.edit_json(path, pointer, Some(value), true)
            }
            (Operation::SetJsonValue { path, pointer, .. }, ResourceState::Absent) => {
                self.edit_json(path, pointer, None, false)
            }
            (op, target) => Err(format!(
                "Tintcast can't put {} back to {target:?}",
                op.resource()
            )),
        }
    }
}
