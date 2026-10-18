use std::collections::{BTreeSet, HashMap};

use crate::{
    DependsOrder, Issue, Scope, TaskDef, TaskFile, TaskKey,
    validate::{Reference, reference, valid_id},
};

/// Both configuration scopes, borrowed. Workspace tasks may reference personal
/// tasks with an explicit `user:<id>`; user tasks never see workspace tasks and
/// unqualified references stay inside the owner's scope.
#[derive(Clone, Copy, Default)]
pub struct Catalog<'a> {
    pub user: Option<&'a TaskFile>,
    pub workspace: Option<&'a TaskFile>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlanNode {
    pub key: TaskKey,
    /// Indices of nodes that must finish (or become ready) first; always lower
    /// than this node's own index.
    pub needs: Vec<usize>,
}

/// A validated dependency DAG with shared nodes run once, in a deterministic
/// topological order. The requested task is the last node.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plan {
    pub nodes: Vec<PlanNode>,
}

impl Plan {
    pub fn root(&self) -> &PlanNode {
        self.nodes.last().expect("a plan always has its root")
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PlanError {
    Unknown {
        from: TaskKey,
        reference: String,
    },
    Cycle(Vec<TaskKey>),
    /// Two parents demand opposite sequential orders for the same tasks.
    OrderConflict(Vec<TaskKey>),
}

impl std::fmt::Display for PlanError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let path = |keys: &[TaskKey]| {
            keys.iter()
                .map(|key| key.id.as_str())
                .collect::<Vec<_>>()
                .join(" -> ")
        };
        match self {
            Self::Unknown { from, reference } => {
                write!(
                    f,
                    "task '{}' depends on unknown task '{reference}'",
                    from.id
                )
            }
            Self::Cycle(keys) => write!(f, "dependency cycle: {}", path(keys)),
            Self::OrderConflict(keys) => {
                write!(f, "conflicting sequential order between: {}", path(keys))
            }
        }
    }
}

impl<'a> Catalog<'a> {
    pub fn file(&self, scope: Scope) -> Option<&'a TaskFile> {
        match scope {
            Scope::User => self.user,
            Scope::Workspace => self.workspace,
        }
    }

    pub fn get(&self, key: &TaskKey) -> Option<&'a TaskDef> {
        self.file(key.scope)?.get(&key.id)
    }

    fn resolve(&self, owner: &TaskKey, dependency: &str) -> Option<TaskKey> {
        let key = match reference(dependency) {
            Reference::SameScope(id) => TaskKey::new(owner.scope, id),
            Reference::User(id) => TaskKey::new(Scope::User, id),
        };
        (valid_id(&key.id) && self.get(&key).is_some()).then_some(key)
    }

    /// Cross-scope checks that a single file cannot make: every `user:` reference
    /// must name an existing personal task.
    pub fn validate(&self) -> Vec<Issue> {
        let mut issues = Vec::new();
        for scope in [Scope::User, Scope::Workspace] {
            let Some(file) = self.file(scope) else {
                continue;
            };
            for task in &file.tasks {
                for dependency in &task.depends_on {
                    if let Reference::User(id) = reference(dependency)
                        && valid_id(id)
                        && self.user.is_none_or(|user| user.get(id).is_none())
                    {
                        issues.push(Issue::task(
                            &task.id,
                            "depends_on",
                            format!("unknown personal task '{id}'"),
                        ));
                    }
                }
            }
        }
        issues
    }

    /// The default task of `group`: the workspace default wins over the user's.
    pub fn default_for_group(&self, group: &str) -> Option<TaskKey> {
        [Scope::Workspace, Scope::User]
            .into_iter()
            .find_map(|scope| {
                self.file(scope)?
                    .tasks
                    .iter()
                    .find(|task| task.is_default && task.group.as_deref() == Some(group))
                    .map(|task| TaskKey::new(scope, &task.id))
            })
    }

    /// Resolve the complete graph before anything launches.
    pub fn plan(&self, root: &TaskKey) -> Result<Plan, PlanError> {
        let mut graph = Graph::default();
        self.discover(root, &mut graph, &mut Vec::new())?;
        // Sequential parents also order their children among themselves.
        let mut edges = graph.edges.clone();
        for key in &graph.order {
            let task = self.get(key).expect("discovered tasks exist");
            if task.effective_depends_order() == DependsOrder::Sequence {
                let children = graph.children(self, key);
                for pair in children.windows(2) {
                    edges
                        .entry(pair[1].clone())
                        .or_default()
                        .insert(pair[0].clone());
                }
            }
        }
        let order = topological(&graph.order, &edges)?;
        let index: HashMap<&TaskKey, usize> =
            order.iter().enumerate().map(|(i, key)| (key, i)).collect();
        let nodes = order
            .iter()
            .map(|key| PlanNode {
                key: key.clone(),
                needs: edges
                    .get(key)
                    .map(|set| set.iter().map(|need| index[need]).collect())
                    .unwrap_or_default(),
            })
            .collect();
        Ok(Plan { nodes })
    }

    fn discover(
        &self,
        key: &TaskKey,
        graph: &mut Graph,
        stack: &mut Vec<TaskKey>,
    ) -> Result<(), PlanError> {
        if let Some(start) = stack.iter().position(|entry| entry == key) {
            let mut path = stack[start..].to_vec();
            path.push(key.clone());
            return Err(PlanError::Cycle(path));
        }
        if graph.edges.contains_key(key) {
            return Ok(());
        }
        let task = self.get(key).expect("callers resolve before descending");
        graph.edges.insert(key.clone(), BTreeSet::new());
        stack.push(key.clone());
        for dependency in &task.depends_on {
            let child = self
                .resolve(key, dependency)
                .ok_or_else(|| PlanError::Unknown {
                    from: key.clone(),
                    reference: dependency.clone(),
                })?;
            graph.edges.get_mut(key).unwrap().insert(child.clone());
            self.discover(&child, graph, stack)?;
        }
        stack.pop();
        graph.order.push(key.clone());
        Ok(())
    }
}

#[derive(Default)]
struct Graph {
    edges: HashMap<TaskKey, BTreeSet<TaskKey>>,
    /// Discovery (post-)order, used for deterministic tie-breaking.
    order: Vec<TaskKey>,
}

impl Graph {
    fn children(&self, catalog: &Catalog<'_>, key: &TaskKey) -> Vec<TaskKey> {
        catalog
            .get(key)
            .map(|task| {
                task.depends_on
                    .iter()
                    .filter_map(|dependency| catalog.resolve(key, dependency))
                    .collect()
            })
            .unwrap_or_default()
    }
}

fn topological(
    discovered: &[TaskKey],
    edges: &HashMap<TaskKey, BTreeSet<TaskKey>>,
) -> Result<Vec<TaskKey>, PlanError> {
    let mut remaining: Vec<&TaskKey> = discovered.iter().collect();
    let mut done: Vec<TaskKey> = Vec::new();
    while !remaining.is_empty() {
        let position = remaining
            .iter()
            .position(|key| edges[*key].iter().all(|need| done.contains(need)));
        match position {
            Some(position) => done.push(remaining.remove(position).clone()),
            None => {
                return Err(PlanError::OrderConflict(
                    remaining.into_iter().cloned().collect(),
                ));
            }
        }
    }
    Ok(done)
}
