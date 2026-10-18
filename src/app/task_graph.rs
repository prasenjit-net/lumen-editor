//! Task groups: a task with dependencies (or a compound task) runs as an
//! invocation. The whole graph is validated and resolved before anything starts;
//! a scheduler then starts nodes as their prerequisites succeed, within the
//! concurrency limit, applies the group's failure policy, and reports progress in
//! the group's own summary tab. Child runs are ordinary runs with their own tabs.
use super::{
    LumenApp,
    task_runs::{
        InputMap, InputPrompt, MAX_ACTIVE_RUNS, Readiness, RunId, RunOutcome, RunState, StopReason,
        TaskRun, command_line,
    },
};
use crate::model::task_output::{RunLog, Stream};
use eframe::egui;
use lumen_task_core::{
    Catalog, FailurePolicy, ResolvedTask, SavePolicy, Scope, TaskFile, TaskKey, TaskKind,
};
use std::time::Instant;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct InvocationId(pub(super) u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NodeStatus {
    Pending,
    Running(RunId),
    /// A background task that printed its readiness text and is still running:
    /// dependents may start, and the group stays open until it ends.
    Serving(RunId),
    Succeeded,
    Failed,
    Skipped,
}

/// One task of a validated graph. `resolved` is `None` for a compound node,
/// which only orders its dependencies.
#[derive(Clone)]
pub(crate) struct PlannedNode {
    pub(crate) key: TaskKey,
    pub(crate) label: String,
    pub(crate) needs: Vec<usize>,
    pub(crate) resolved: Option<ResolvedTask>,
    pub(crate) digest: String,
    pub(crate) save: SavePolicy,
    pub(crate) single: bool,
}

/// What planning a run produced.
pub(crate) enum PlanResult {
    Ready(StartPlan),
    /// Declared inputs still without a value; ask before resolving anything.
    NeedsInputs(Vec<InputPrompt>),
}

/// Everything needed to start a run or a group, fixed before the first process.
#[derive(Clone)]
pub(crate) struct StartPlan {
    pub(crate) root: TaskKey,
    pub(crate) label: String,
    pub(crate) nodes: Vec<PlannedNode>,
    pub(crate) policy: FailurePolicy,
    pub(crate) root_single: bool,
}

impl StartPlan {
    /// The strictest save policy among the tasks that will run.
    pub(crate) fn save_policy(&self) -> SavePolicy {
        let mut result = SavePolicy::None;
        for node in self.nodes.iter().filter(|node| node.resolved.is_some()) {
            match node.save {
                SavePolicy::Save => return SavePolicy::Save,
                SavePolicy::Prompt => result = SavePolicy::Prompt,
                SavePolicy::None => {}
            }
        }
        result
    }

    fn is_single_run(&self) -> bool {
        self.nodes.len() == 1 && self.nodes[0].resolved.is_some()
    }
}

struct Node {
    planned: PlannedNode,
    status: NodeStatus,
}

pub(crate) struct Invocation {
    pub(crate) id: InvocationId,
    pub(crate) parent: RunId,
    nodes: Vec<Node>,
    policy: FailurePolicy,
    /// Set once the group is winding down (a stop request or a fail-fast failure).
    stopping: Option<StopReason>,
    first_failure: Option<String>,
    /// Whether the "only services left" notice was logged already.
    announced_serving: bool,
}

/// A prerequisite that lets dependents start.
fn satisfied(status: NodeStatus) -> bool {
    matches!(status, NodeStatus::Succeeded | NodeStatus::Serving(_))
}

impl LumenApp {
    fn load_task_files(&self) -> Result<(Option<TaskFile>, Option<TaskFile>), String> {
        let mut files = [None, None];
        for (slot, scope) in [(0, Scope::User), (1, Scope::Workspace)] {
            let Ok((path, _)) = self.task_config_path(scope) else {
                continue;
            };
            let loaded = crate::services::tasks::load(self.settings_store.as_ref(), &path)?;
            if loaded.text.is_none() {
                continue;
            }
            files[slot] = Some(loaded.parsed.map_err(|issues| {
                format!(
                    "The {} tasks file has problems ({}); open it as TOML to fix them",
                    scope.label().to_lowercase(),
                    issues.first().map(ToString::to_string).unwrap_or_default()
                )
            })?);
        }
        let [user, workspace] = files;
        Ok((user, workspace))
    }

    /// Validate the dependency graph and resolve every task in it, from disk, so a
    /// stale sidebar can never run an outdated command. Any problem aborts the
    /// whole run before a process starts.
    pub(super) fn plan_for_run(
        &self,
        key: &TaskKey,
        inputs: &InputMap,
    ) -> Result<PlanResult, String> {
        let (user, workspace) = self.load_task_files()?;
        let catalog = Catalog {
            user: user.as_ref(),
            workspace: workspace.as_ref(),
        };
        let root_def = catalog
            .get(key)
            .ok_or_else(|| format!("Task '{}' no longer exists", key.id))?;
        let plan = catalog
            .plan(key)
            .map_err(|error| format!("Cannot run '{}': {error}", root_def.label))?;
        // Ask for declared inputs before resolving: nothing starts on a cancelled prompt.
        let mut needed: Vec<InputPrompt> = Vec::new();
        for node in &plan.nodes {
            let def = catalog.get(&node.key).expect("planned tasks exist");
            for id in lumen_task_core::input_references(def) {
                let slot = (node.key.scope, id.clone());
                if inputs.contains_key(&slot)
                    || needed.iter().any(|p| (p.scope, &p.id) == (slot.0, &slot.1))
                {
                    continue;
                }
                let Some(decl) = catalog
                    .file(node.key.scope)
                    .and_then(|file| file.input(&id))
                else {
                    return Err(format!(
                        "Cannot run '{}': input '{id}' is not declared",
                        root_def.label
                    ));
                };
                let value = match decl.kind {
                    lumen_task_core::InputKind::Choice => decl
                        .default
                        .clone()
                        .or_else(|| decl.options.first().cloned())
                        .unwrap_or_default(),
                    lumen_task_core::InputKind::Text => decl.default.clone().unwrap_or_default(),
                    lumen_task_core::InputKind::Secret => String::new(),
                };
                needed.push(InputPrompt {
                    scope: node.key.scope,
                    id,
                    prompt: decl.prompt.clone(),
                    kind: decl.kind,
                    options: decl.options.clone(),
                    value,
                });
            }
        }
        if !needed.is_empty() {
            return Ok(PlanResult::NeedsInputs(needed));
        }
        // Services keep their slot for as long as they run; a graph whose services
        // leave no room for the tasks that need them could never finish.
        let services = plan
            .nodes
            .iter()
            .filter(|node| catalog.get(&node.key).is_some_and(|d| d.is_background))
            .count();
        let foreground = plan
            .nodes
            .iter()
            .filter(|node| {
                catalog
                    .get(&node.key)
                    .is_some_and(|d| d.kind != TaskKind::Compound && !d.is_background)
            })
            .count();
        if services >= MAX_ACTIVE_RUNS || (foreground > 0 && services + 1 > MAX_ACTIVE_RUNS) {
            return Err(format!(
                "Cannot run '{}': it needs {services} background task(s) running at once and \
                 only {MAX_ACTIVE_RUNS} processes can run, which would leave no room for the \
                 rest",
                root_def.label
            ));
        }
        let mut nodes = Vec::with_capacity(plan.nodes.len());
        for node in &plan.nodes {
            let def = catalog.get(&node.key).expect("planned tasks exist");
            let executable = def.kind != TaskKind::Compound;
            let resolved = if executable {
                Some(
                    self.resolve_for_run(&node.key, def, inputs)
                        .map_err(|message| {
                            format!(
                                "Cannot run '{}': '{}': {message}",
                                root_def.label, def.label
                            )
                        })?,
                )
            } else {
                None
            };
            nodes.push(PlannedNode {
                key: node.key.clone(),
                label: def.label.clone(),
                needs: node.needs.clone(),
                resolved,
                digest: lumen_task_core::execution_digest(def),
                save: def.effective_save(),
                single: def.effective_instance_policy() == lumen_task_core::InstancePolicy::Single,
            });
        }
        Ok(PlanResult::Ready(StartPlan {
            root: key.clone(),
            label: root_def.label.clone(),
            nodes,
            policy: root_def.effective_failure_policy(),
            root_single: root_def.effective_instance_policy()
                == lumen_task_core::InstancePolicy::Single,
        }))
    }

    pub(super) fn start_plan(&mut self, plan: StartPlan, ctx: &egui::Context) {
        if plan.is_single_run() {
            let node = plan.nodes.into_iter().next().expect("one node");
            let resolved = node.resolved.expect("checked by is_single_run");
            self.start_run(node.key, resolved, ctx, true, None);
        } else {
            self.start_invocation(plan, ctx);
        }
    }

    fn start_invocation(&mut self, plan: StartPlan, ctx: &egui::Context) {
        // A rerun replaces the group's finished summary tab.
        let stale: Vec<RunId> = self
            .view
            .task_runs
            .runs()
            .iter()
            .filter(|run| run.summary && !run.is_active() && run.key == plan.root)
            .map(|run| run.id)
            .collect();
        for id in stale {
            self.view.task_runs.remove_finished(id);
        }
        let id = InvocationId(self.view.task_runs.next_invocation);
        self.view.task_runs.next_invocation += 1;
        let order = plan
            .nodes
            .iter()
            .map(|node| node.label.as_str())
            .collect::<Vec<_>>()
            .join(" → ");
        let mut parent = TaskRun::summary(
            plan.root.clone(),
            plan.label.clone(),
            order,
            self.editor.workspace.clone(),
            id,
            Instant::now(),
        );
        parent.log.push(
            Stream::Stdout,
            format!(
                "Task group '{}': {} task(s)\n",
                plan.label,
                plan.nodes.len()
            )
            .as_bytes(),
        );
        let parent_id = self.view.task_runs.insert(parent);
        self.view.task_runs.last = Some(plan.root.clone());
        self.view.task_runs.selected = Some(parent_id);
        self.show_task_run(parent_id);
        self.view.status = format!("Running '{}'", plan.label);
        self.view.task_runs.invocations.push(Invocation {
            id,
            parent: parent_id,
            nodes: plan
                .nodes
                .into_iter()
                .map(|planned| Node {
                    planned,
                    status: NodeStatus::Pending,
                })
                .collect(),
            policy: plan.policy,
            stopping: None,
            first_failure: None,
            announced_serving: false,
        });
        self.advance_invocations(Instant::now(), ctx);
    }

    /// Ask a group to wind down: running children are stopped and nothing new
    /// starts. The summary finishes once the children have.
    pub(super) fn stop_invocation(&mut self, parent: RunId, reason: StopReason) {
        let Some(position) = self
            .view
            .task_runs
            .invocations
            .iter()
            .position(|inv| inv.parent == parent)
        else {
            return;
        };
        let inv = &mut self.view.task_runs.invocations[position];
        inv.stopping.get_or_insert(reason);
        let running: Vec<RunId> = inv
            .nodes
            .iter()
            .filter_map(|node| match node.status {
                NodeStatus::Running(id) | NodeStatus::Serving(id) => Some(id),
                _ => None,
            })
            .collect();
        if let Some(run) = self.view.task_runs.get_mut(parent) {
            run.state = RunState::Stopping;
        }
        for id in running {
            self.stop_task_run(id, StopReason::Group);
        }
    }

    fn log_group(&mut self, parent: RunId, text: &str) {
        if let Some(run) = self.view.task_runs.get_mut(parent) {
            run.log.push(Stream::Stdout, format!("{text}\n").as_bytes());
        }
    }

    /// Per-frame scheduler step for every live group. Returns the root keys of
    /// groups that finished stopped with a pending restart.
    pub(super) fn advance_invocations(&mut self, now: Instant, ctx: &egui::Context) {
        let mut restarts = Vec::new();
        let mut index = 0;
        while index < self.view.task_runs.invocations.len() {
            if self.advance_invocation(index, now, ctx, &mut restarts) {
                self.view.task_runs.invocations.remove(index);
            } else {
                index += 1;
            }
        }
        for key in restarts {
            self.run_task(key, ctx);
        }
    }

    /// Returns true when the group finished and can be dropped.
    fn advance_invocation(
        &mut self,
        index: usize,
        now: Instant,
        ctx: &egui::Context,
        restarts: &mut Vec<TaskKey>,
    ) -> bool {
        let parent = self.view.task_runs.invocations[index].parent;
        if self.view.task_runs.get(parent).is_none() {
            // The summary tab was removed (for example by quitting): drop the group.
            return true;
        }
        loop {
            let mut changed = false;
            // 1. Learn how running children are doing: ready, ended, or gone.
            let watching: Vec<(usize, RunId, bool)> = self.view.task_runs.invocations[index]
                .nodes
                .iter()
                .enumerate()
                .filter_map(|(i, node)| match node.status {
                    NodeStatus::Running(id) => Some((i, id, false)),
                    NodeStatus::Serving(id) => Some((i, id, true)),
                    _ => None,
                })
                .collect();
            for (node_index, run_id, was_serving) in watching {
                let outcome = match self.view.task_runs.get(run_id) {
                    Some(run) if run.is_active() => {
                        if !was_serving && matches!(run.readiness, Readiness::Ready) {
                            let label = {
                                let node =
                                    &mut self.view.task_runs.invocations[index].nodes[node_index];
                                node.status = NodeStatus::Serving(run_id);
                                node.planned.label.clone()
                            };
                            self.log_group(parent, &format!("[ready] {label}"));
                            changed = true;
                        }
                        continue;
                    }
                    Some(run) => run.outcome().cloned().unwrap_or(RunOutcome::Cancelled),
                    None => RunOutcome::GroupFailed("its tab was closed".into()),
                };
                let (label, ok, note) = {
                    let inv = &mut self.view.task_runs.invocations[index];
                    let others_busy = inv.nodes.iter().enumerate().any(|(i, node)| {
                        i != node_index
                            && matches!(node.status, NodeStatus::Pending | NodeStatus::Running(_))
                    });
                    let winding = inv.stopping.is_some();
                    let node = &mut inv.nodes[node_index];
                    // A service that is stopped by the group, or that simply ends once
                    // nothing else needs it, is fine; one that dies while dependents are
                    // still running (or exits badly) is a failure.
                    let ok = if was_serving {
                        winding || (outcome.is_success() && !others_busy)
                    } else {
                        outcome.is_success()
                    };
                    node.status = if ok {
                        NodeStatus::Succeeded
                    } else {
                        NodeStatus::Failed
                    };
                    if !ok && inv.first_failure.is_none() {
                        inv.first_failure = Some(node.planned.label.clone());
                    }
                    let note = if was_serving && !ok && outcome.is_success() {
                        "stopped while other tasks still needed it".to_string()
                    } else {
                        outcome.summary()
                    };
                    (node.planned.label.clone(), ok, note)
                };
                self.log_group(
                    parent,
                    &format!("[{}] {label}: {note}", if ok { "ok" } else { "failed" }),
                );
                changed = true;
            }
            // 2. Fail fast: stop the siblings still running.
            let fail_fast = {
                let inv = &mut self.view.task_runs.invocations[index];
                let trigger = inv.first_failure.is_some()
                    && inv.policy == FailurePolicy::FailFast
                    && inv.stopping.is_none();
                if trigger {
                    inv.stopping = Some(StopReason::Group);
                }
                trigger
            };
            if fail_fast {
                let running: Vec<RunId> = self.view.task_runs.invocations[index]
                    .nodes
                    .iter()
                    .filter_map(|node| match node.status {
                        NodeStatus::Running(id) | NodeStatus::Serving(id) => Some(id),
                        _ => None,
                    })
                    .collect();
                for id in running {
                    self.stop_task_run(id, StopReason::Group);
                }
                changed = true;
            }
            // 3. Skip what can no longer run, and complete compound nodes.
            let mut skipped = Vec::new();
            {
                let inv = &mut self.view.task_runs.invocations[index];
                for i in 0..inv.nodes.len() {
                    if inv.nodes[i].status != NodeStatus::Pending {
                        continue;
                    }
                    let blocked = inv.nodes[i].planned.needs.iter().any(|need| {
                        matches!(
                            inv.nodes[*need].status,
                            NodeStatus::Failed | NodeStatus::Skipped
                        )
                    });
                    if inv.stopping.is_some() || blocked {
                        inv.nodes[i].status = NodeStatus::Skipped;
                        skipped.push(inv.nodes[i].planned.label.clone());
                        changed = true;
                    } else if inv.nodes[i].planned.resolved.is_none()
                        && inv.nodes[i]
                            .planned
                            .needs
                            .iter()
                            .all(|need| satisfied(inv.nodes[*need].status))
                    {
                        inv.nodes[i].status = NodeStatus::Succeeded;
                        changed = true;
                    }
                }
            }
            for label in skipped {
                self.log_group(parent, &format!("[skipped] {label}"));
            }
            // 4. Start ready tasks within the concurrency limit.
            let mut to_start = Vec::new();
            {
                let inv = &self.view.task_runs.invocations[index];
                if inv.stopping.is_none() {
                    let mut slots =
                        MAX_ACTIVE_RUNS.saturating_sub(self.view.task_runs.active_process_count());
                    for (i, node) in inv.nodes.iter().enumerate() {
                        if slots == 0 {
                            break;
                        }
                        if node.status == NodeStatus::Pending
                            && node.planned.resolved.is_some()
                            && node
                                .planned
                                .needs
                                .iter()
                                .all(|need| satisfied(inv.nodes[*need].status))
                        {
                            to_start.push(i);
                            slots -= 1;
                        }
                    }
                }
            }
            for node_index in to_start {
                let inv_id = self.view.task_runs.invocations[index].id;
                let planned = self.view.task_runs.invocations[index].nodes[node_index]
                    .planned
                    .clone();
                let resolved = planned
                    .resolved
                    .clone()
                    .expect("only executable nodes start");
                self.log_group(
                    parent,
                    &format!("[start] {}: {}", planned.label, command_line(&resolved)),
                );
                let run_id = self.start_run(planned.key, resolved, ctx, false, Some(inv_id));
                self.view.task_runs.invocations[index].nodes[node_index].status =
                    NodeStatus::Running(run_id);
                changed = true;
            }
            if !changed {
                break;
            }
        }
        // Only services are left: say so once, since the group stays open for them.
        {
            let inv = &mut self.view.task_runs.invocations[index];
            let only_services = !inv.announced_serving
                && inv.stopping.is_none()
                && inv
                    .nodes
                    .iter()
                    .any(|n| matches!(n.status, NodeStatus::Serving(_)))
                && inv.nodes.iter().all(|n| {
                    matches!(
                        n.status,
                        NodeStatus::Succeeded
                            | NodeStatus::Failed
                            | NodeStatus::Skipped
                            | NodeStatus::Serving(_)
                    )
                });
            if only_services {
                inv.announced_serving = true;
                self.log_group(
                    parent,
                    "All tasks are done; background tasks are still running. Stop the group to end them.",
                );
            }
        }
        // 5. Finished when nothing runs and nothing can still start.
        let done = self.view.task_runs.invocations[index]
            .nodes
            .iter()
            .all(|node| {
                matches!(
                    node.status,
                    NodeStatus::Succeeded | NodeStatus::Failed | NodeStatus::Skipped
                )
            });
        if !done {
            return false;
        }
        let (stopping, failure, skipped_any) = {
            let inv = &self.view.task_runs.invocations[index];
            (
                inv.stopping,
                inv.first_failure.clone(),
                inv.nodes.iter().any(|n| n.status == NodeStatus::Skipped),
            )
        };
        let outcome = match (stopping, failure) {
            (Some(StopReason::Group), Some(label)) | (None, Some(label)) => {
                RunOutcome::GroupFailed(label)
            }
            (Some(StopReason::Group), None) => RunOutcome::Cancelled,
            // Stopping a group whose work all finished (only its services were
            // left) is a normal end; stopping one with work still undone is a stop.
            (Some(_), _) if !skipped_any => RunOutcome::Completed,
            (Some(_), _) => RunOutcome::Cancelled,
            (None, None) => RunOutcome::Completed,
        };
        let label = self
            .view
            .task_runs
            .get(parent)
            .map(|run| run.label.clone())
            .unwrap_or_default();
        self.log_group(
            parent,
            &format!("Task group '{label}': {}", outcome.summary()),
        );
        self.view.status = format!("'{label}': {}", outcome.summary());
        let mut restart_key = None;
        if let Some(run) = self.view.task_runs.get_mut(parent) {
            run.log.finish();
            run.ended = Some(now);
            if run.restart_after && stopping == Some(StopReason::Restart) {
                restart_key = Some(run.key.clone());
            }
            run.state = RunState::Finished(outcome);
        }
        if let Some(key) = restart_key {
            restarts.push(key);
        }
        true
    }
}

impl TaskRun {
    /// A task group's own tab.
    pub(super) fn summary(
        key: TaskKey,
        label: String,
        order: String,
        workspace: Option<std::path::PathBuf>,
        invocation: InvocationId,
        now: Instant,
    ) -> Self {
        Self::new_summary(
            key,
            label,
            order,
            workspace,
            invocation,
            now,
            RunLog::new(1 << 20),
        )
    }
}
