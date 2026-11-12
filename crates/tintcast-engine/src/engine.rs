use std::collections::BTreeMap;
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
use crate::services::{CommitError, DISPLAY_NOT_CONNECTED, RegistryChange, SystemServices};
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
            Self::Failed(error) if error == DISPLAY_NOT_CONNECTED => {
                format!("{DISPLAY_NOT_CONNECTED}: its wallpaper will be restored when it's back.")
            }
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

type StepFailure = (Option<String>, String);

type Snapshot = BTreeMap<String, Result<ResourceState, String>>;

struct Prepared {
    resource: String,
    before: ResourceState,
    before_bytes: Option<Option<Vec<u8>>>,
    file_backup: Option<String>,
    previous: Option<LedgerEntry>,
}

struct Reversal<'a> {
    integration: &'a str,
    op: &'a Operation,
    current: &'a ResourceState,
    target: &'a ResourceState,
    file_backup: Option<&'a str>,
    before_bytes: Option<&'a Option<Vec<u8>>>,
}

impl Reversal<'_> {
    fn registry_change(&self) -> Option<RegistryChange> {
        self.op.registry_change(self.target)
    }
}

enum Decision {
    Settled(RestoreResult),
    Reverse(usize),
}

impl Decision {
    fn settle(self, outcomes: &[Result<(), String>]) -> RestoreResult {
        match self {
            Self::Settled(result) => result,
            Self::Reverse(index) => match &outcomes[index] {
                Ok(()) => RestoreResult::Restored,
                Err(error) => RestoreResult::Failed(error.clone()),
            },
        }
    }
}

fn is_registry(op: &Operation) -> bool {
    matches!(op, Operation::SetRegistryValue { .. })
}

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
        let snapshot = self.snapshot(&planned);
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
                        &snapshot,
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

    fn snapshot(&self, planned: &[PlannedIntegration]) -> Snapshot {
        planned
            .iter()
            .filter_map(|item| match &item.outcome {
                PlannedOutcome::Changes { operations, .. } => Some(operations),
                _ => None,
            })
            .flatten()
            .map(|op| (op.resource(), self.state_of(op)))
            .collect()
    }

    #[allow(clippy::too_many_arguments)]
    fn apply_integration(
        &self,
        ledger: &mut Ledger,
        run: &mut JournalRun,
        integration: &dyn Integration,
        operations: &[Operation],
        notes: &[String],
        snapshot: &Snapshot,
        applied: &mut Vec<AppliedValue>,
    ) -> IntegrationResult {
        let mut done: Vec<Done> = Vec::new();
        let mut failure = None;
        for group in operations.chunk_by(|a, b| is_registry(a) && is_registry(b)) {
            match self.apply_group(ledger, run, integration.id(), group, snapshot) {
                Ok(steps) => done.extend(steps),
                Err(error) => {
                    failure = Some(error);
                    break;
                }
            }
        }
        let summaries = operations.iter().map(Operation::summary).collect();
        let mut notes = notes.to_vec();
        if let Some((operation, error)) = failure {
            let rollback_errors = self.roll_back_done(ledger, run, integration.id(), &done);
            return IntegrationResult {
                id: integration.id().to_owned(),
                name: integration.name().to_owned(),
                outcome: Outcome::Failed {
                    operation,
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

    fn prepare(
        &self,
        ledger: &Ledger,
        op: &Operation,
        snapshot: &Snapshot,
    ) -> Result<Prepared, String> {
        let resource = op.resource();
        let before = match snapshot.get(&resource) {
            Some(state) => state.clone()?,
            None => self.state_of(op)?,
        };
        let before_bytes = match op.file_path() {
            Some(path) => Some(self.services.read_file(path).map_err(|e| e.to_string())?),
            None => None,
        };
        let file_backup = match &before_bytes {
            Some(Some(bytes)) => Some(self.store.backup(bytes).map_err(|e| e.to_string())?),
            _ => None,
        };
        Ok(Prepared {
            previous: ledger.get(&resource).cloned(),
            resource,
            before,
            before_bytes,
            file_backup,
        })
    }

    fn apply_group(
        &self,
        ledger: &mut Ledger,
        run: &mut JournalRun,
        integration: &str,
        group: &[Operation],
        snapshot: &Snapshot,
    ) -> Result<Vec<Done>, StepFailure> {
        let mut prepared = Vec::with_capacity(group.len());
        for op in group {
            let item = self
                .prepare(ledger, op, snapshot)
                .map_err(|error| (Some(op.summary()), error))?;
            prepared.push(item);
        }
        let first_summary = group.first().map(Operation::summary);
        let put_back = |ledger: &mut Ledger| {
            for item in prepared.iter().rev() {
                match &item.previous {
                    Some(previous) => ledger.insert(previous.clone()),
                    None => {
                        ledger.remove(&item.resource);
                    }
                }
            }
        };
        let now = Timestamp::now();
        for (op, item) in group.iter().zip(&prepared) {
            let target = op.target_state();
            match ledger.get_mut(&item.resource) {
                Some(entry) => entry.pending = Some(target),
                None => ledger.insert(LedgerEntry {
                    sequence: ledger.next_sequence(),
                    resource: item.resource.clone(),
                    integration: integration.to_owned(),
                    operation: op.clone(),
                    original: item.before.clone(),
                    applied: item.before.clone(),
                    pending: Some(target),
                    file_backup: item.file_backup.clone(),
                    first_touched: now,
                    last_applied: now,
                    error: None,
                }),
            }
        }
        if let Err(error) = self.store.save_ledger(ledger) {
            put_back(ledger);
            return Err((first_summary, error.to_string()));
        }
        let abandon = |ledger: &mut Ledger| {
            put_back(ledger);
            let _ = self.store.save_ledger(ledger);
        };
        let first_step = run.steps.len();
        for (op, item) in group.iter().zip(&prepared) {
            run.steps.push(JournalStep {
                integration: integration.to_owned(),
                resource: item.resource.clone(),
                summary: op.summary(),
                operation: op.clone(),
                before: item.before.clone(),
                file_backup: item.file_backup.clone(),
                state: StepState::Pending,
            });
        }
        let mark = |run: &mut JournalRun, state: StepState| {
            for step in &mut run.steps[first_step..] {
                step.state = state;
            }
        };
        if let Err(error) = self.store.save_journal(run) {
            mark(run, StepState::Failed);
            abandon(ledger);
            return Err((first_summary, error.to_string()));
        }
        let applied = match self.execute_group(group) {
            Ok(applied) => applied,
            Err((index, error)) => {
                mark(run, StepState::Failed);
                let _ = self.store.save_journal(run);
                abandon(ledger);
                let operation = index.and_then(|i| group.get(i)).map(Operation::summary);
                return Err((operation, error));
            }
        };
        let now = Timestamp::now();
        for ((op, item), state) in group.iter().zip(&prepared).zip(&applied) {
            if let Some(entry) = ledger.get_mut(&item.resource) {
                entry.applied = state.clone();
                entry.pending = None;
                entry.operation = op.clone();
                entry.integration = integration.to_owned();
                entry.last_applied = now;
                entry.error = None;
            }
        }
        if let Err(error) = self.store.save_ledger(ledger) {
            let reversals: Vec<_> = group
                .iter()
                .zip(&prepared)
                .zip(&applied)
                .rev()
                .map(|((op, item), state)| Reversal {
                    integration,
                    op,
                    current: state,
                    target: &item.before,
                    file_backup: None,
                    before_bytes: item.before_bytes.as_ref(),
                })
                .collect();
            let _ = self.reverse(&reversals);
            abandon(ledger);
            mark(run, StepState::Failed);
            let _ = self.store.save_journal(run);
            return Err((first_summary, error.to_string()));
        }
        mark(run, StepState::Applied);
        let _ = self.store.save_journal(run);
        Ok(group
            .iter()
            .zip(prepared)
            .zip(applied)
            .enumerate()
            .map(|(offset, ((op, item), applied))| Done {
                op: op.clone(),
                applied,
                first_touch: item.previous.is_none(),
                before: item.before,
                before_bytes: item.before_bytes,
                step: first_step + offset,
            })
            .collect())
    }

    fn execute_group(
        &self,
        group: &[Operation],
    ) -> Result<Vec<ResourceState>, (Option<usize>, String)> {
        let changes: Option<Vec<RegistryChange>> = group
            .iter()
            .map(|op| op.registry_change(&op.target_state()))
            .collect();
        match changes {
            Some(changes) => self
                .services
                .commit_registry(&changes)
                .map(|()| group.iter().map(Operation::target_state).collect())
                .map_err(|error| (error.index, error.error.to_string())),
            None => group
                .iter()
                .enumerate()
                .map(|(index, op)| self.execute(op).map_err(|error| (Some(index), error)))
                .collect(),
        }
    }

    fn roll_back_done(
        &self,
        ledger: &mut Ledger,
        run: &mut JournalRun,
        integration: &str,
        done: &[Done],
    ) -> Vec<String> {
        let reversals: Vec<_> = done
            .iter()
            .rev()
            .map(|item| Reversal {
                integration,
                op: &item.op,
                current: &item.applied,
                target: &item.before,
                file_backup: None,
                before_bytes: item.before_bytes.as_ref(),
            })
            .collect();
        let outcomes = self.reverse(&reversals);
        let mut errors = Vec::new();
        for (item, outcome) in done.iter().rev().zip(outcomes) {
            match outcome {
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

    fn reverse(&self, items: &[Reversal<'_>]) -> Vec<Result<(), String>> {
        let mut results: Vec<Option<Result<(), String>>> = vec![None; items.len()];
        for (index, item) in items.iter().enumerate() {
            if results[index].is_some() {
                continue;
            }
            if item.registry_change().is_none() {
                results[index] = Some(self.reverse_one(item));
                continue;
            }
            let batch: Vec<usize> = (index..items.len())
                .filter(|&i| {
                    items[i].integration == item.integration && items[i].registry_change().is_some()
                })
                .collect();
            let members: Vec<&Reversal<'_>> = batch.iter().map(|&i| &items[i]).collect();
            for (i, outcome) in batch.iter().zip(self.commit_reversals(&members)) {
                results[*i] = Some(outcome);
            }
        }
        results
            .into_iter()
            .map(|result| result.unwrap_or(Ok(())))
            .collect()
    }

    fn commit_reversals(&self, items: &[&Reversal<'_>]) -> Vec<Result<(), String>> {
        let mut results: Vec<Option<Result<(), String>>> = vec![None; items.len()];
        loop {
            let open: Vec<usize> = (0..items.len()).filter(|&i| results[i].is_none()).collect();
            let changes: Vec<RegistryChange> = open
                .iter()
                .filter_map(|&i| items[i].registry_change())
                .collect();
            match self.services.commit_registry(&changes) {
                Ok(()) => {
                    for i in open {
                        results[i] = Some(Ok(()));
                    }
                    break;
                }
                Err(CommitError {
                    index: Some(failed),
                    error,
                }) if failed < open.len() => {
                    results[open[failed]] = Some(Err(error.to_string()));
                }
                Err(error) => {
                    for i in open {
                        results[i] = Some(Err(error.to_string()));
                    }
                    break;
                }
            }
        }
        results
            .into_iter()
            .map(|result| result.unwrap_or(Ok(())))
            .collect()
    }

    fn reverse_one(&self, item: &Reversal<'_>) -> Result<(), String> {
        if let Some(bytes) = item.before_bytes {
            return self.revert(item.op, item.target, Some(bytes));
        }
        if self.restore_json_bytes(item.op, item.current, item.file_backup) {
            return Ok(());
        }
        self.revert(item.op, item.target, None)
            .and_then(|()| self.tidy_json(item.op, item.target, item.file_backup))
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
        let entries = ledger.newest_first();
        let currents: Vec<_> = entries
            .iter()
            .map(|entry| self.state_of(&entry.operation))
            .collect();
        let mut reversals = Vec::new();
        let mut decisions = Vec::with_capacity(entries.len());
        for (entry, current) in entries.iter().zip(&currents) {
            decisions.push(match current {
                Err(error) => Decision::Settled(RestoreResult::Failed(error.clone())),
                Ok(current) if *current == entry.original => {
                    Decision::Settled(RestoreResult::Unchanged)
                }
                Ok(current)
                    if *current == entry.applied || entry.pending.as_ref() == Some(current) =>
                {
                    reversals.push(Reversal {
                        integration: &entry.integration,
                        op: &entry.operation,
                        current,
                        target: &entry.original,
                        file_backup: entry.file_backup.as_deref(),
                        before_bytes: None,
                    });
                    Decision::Reverse(reversals.len() - 1)
                }
                Ok(_) => Decision::Settled(RestoreResult::KeptYourChange),
            });
        }
        let outcomes = self.reverse(&reversals);
        let mut report = RestoreReport::default();
        let mut touched: Vec<String> = Vec::new();
        for (entry, decision) in entries.iter().zip(decisions) {
            if !touched.contains(&entry.integration) {
                touched.push(entry.integration.clone());
            }
            let result = decision.settle(&outcomes);
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
        let steps: Vec<_> = run
            .steps
            .iter()
            .rev()
            .filter(|s| matches!(s.state, StepState::Applied | StepState::Pending))
            .collect();
        let currents: Vec<_> = steps
            .iter()
            .map(|step| self.state_of(&step.operation))
            .collect();
        let mut reversals = Vec::new();
        let mut decisions = Vec::with_capacity(steps.len());
        for (step, current) in steps.iter().zip(&currents) {
            decisions.push(match current {
                Err(error) => Decision::Settled(RestoreResult::Failed(error.clone())),
                Ok(current) if *current == step.before => {
                    Decision::Settled(RestoreResult::Unchanged)
                }
                Ok(current) => {
                    reversals.push(Reversal {
                        integration: &step.integration,
                        op: &step.operation,
                        current,
                        target: &step.before,
                        file_backup: step.file_backup.as_deref(),
                        before_bytes: None,
                    });
                    Decision::Reverse(reversals.len() - 1)
                }
            });
        }
        let outcomes = self.reverse(&reversals);
        let mut items = Vec::new();
        for (step, decision) in steps.iter().zip(decisions) {
            let result = decision.settle(&outcomes);
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

    fn restore_json_bytes(
        &self,
        op: &Operation,
        current: &ResourceState,
        file_backup: Option<&str>,
    ) -> bool {
        let (Operation::SetJsonValue { path, pointer, .. }, Some(sha)) = (op, file_backup) else {
            return false;
        };
        let applied = match current {
            ResourceState::Json { value } => Some(value),
            ResourceState::Absent => None,
            _ => return false,
        };
        let Ok(backup) = self.store.read_backup(sha) else {
            return false;
        };
        let (Ok(original), Ok(Some(text))) = (std::str::from_utf8(&backup), self.read_text(path))
        else {
            return false;
        };
        let edited_only_here =
            jsonc::set(original, pointer, applied).is_ok_and(|expected| expected == text);
        edited_only_here
            && text.as_bytes() != backup
            && self.services.write_file(path, &backup).is_ok()
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
            Operation::SetRegistryValue { .. } => {
                let change = op.registry_change(&op.target_state());
                return services
                    .commit_registry(change.as_slice())
                    .map(|()| op.target_state())
                    .map_err(|e| e.to_string());
            }
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
        if let Some(change) = op.registry_change(target) {
            return services
                .commit_registry(&[change])
                .map_err(|e| e.to_string());
        }
        match (op, target) {
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
