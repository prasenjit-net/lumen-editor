//! Turn scheduling for agent sessions: one state machine per turn that owns the
//! three resources a turn needs, so they can never deadlock each other.
//!
//! * **Queue** - a session runs one turn at a time; later messages wait their turn.
//! * **Write lease** - one potentially writing turn per workspace context.
//! * **Execution slot** - a global limit on turns that are actually working.
//!
//! A turn takes the lease before the slot and releases both when it ends. While it
//! waits for the user (approval/question) it gives the slot back but keeps the
//! lease, because the agent is mid-edit; answering re-acquires a slot first.
//! Pure state: the controller turns [`Dispatch`]es into backend operations.
use lumen_agent_core::SessionId;
use std::path::{Path, PathBuf};

/// Initial global limit on turns executing at once.
pub(crate) const DEFAULT_SLOTS: usize = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct Ticket(pub(crate) u64);

/// Why a turn is not running yet, in the words the session list shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum WaitReason {
    /// An earlier message of the same session is still being handled.
    PreviousMessage,
    /// Every execution slot is in use.
    NoSlot { executing: usize },
    /// Another session may be editing this folder.
    Lease { holder: SessionId },
    /// The user approved, but no slot is free to continue yet.
    ApprovedWaitingForSlot,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Phase {
    Queued,
    Executing,
    /// Waiting for the user: slot released, lease kept.
    WaitingForUser,
    /// Answered, waiting for a slot to continue.
    Resuming,
}

/// A turn the scheduler has just allowed to start (or continue).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Dispatch {
    pub(crate) ticket: Ticket,
    pub(crate) session: SessionId,
    /// False when this resumes a turn that already started.
    pub(crate) fresh: bool,
}

#[derive(Clone, Debug)]
struct Entry {
    ticket: Ticket,
    session: SessionId,
    workspace: PathBuf,
    /// Potentially writing. Unknown modes count as writing; only a provider-enforced
    /// read-only turn passes `false`.
    writes: bool,
    phase: Phase,
    started: bool,
    holds_lease: bool,
    holds_slot: bool,
}

#[derive(Debug)]
pub(crate) struct Scheduler {
    slots: usize,
    next: u64,
    /// Arrival order; fairness is first come, first served.
    entries: Vec<Entry>,
}

impl Default for Scheduler {
    fn default() -> Self {
        Self::new(DEFAULT_SLOTS)
    }
}

impl Scheduler {
    pub(crate) fn new(slots: usize) -> Self {
        Self {
            slots: slots.max(1),
            next: 1,
            entries: Vec::new(),
        }
    }

    #[cfg(test)]
    pub(crate) fn set_slots(&mut self, slots: usize) {
        self.slots = slots.max(1);
    }

    /// Add a turn. `workspace` must already be the canonical context path.
    pub(crate) fn submit(
        &mut self,
        session: SessionId,
        workspace: PathBuf,
        writes: bool,
    ) -> Ticket {
        let ticket = Ticket(self.next);
        self.next += 1;
        self.entries.push(Entry {
            ticket,
            session,
            workspace,
            writes,
            phase: Phase::Queued,
            started: false,
            holds_lease: false,
            holds_slot: false,
        });
        ticket
    }

    fn executing(&self) -> usize {
        self.entries.iter().filter(|e| e.holds_slot).count()
    }

    fn lease_holder(&self, workspace: &Path, except: Ticket) -> Option<&Entry> {
        self.entries
            .iter()
            .find(|e| e.holds_lease && e.ticket != except && e.workspace == workspace)
    }

    /// The oldest turn of each session is the only one allowed to start.
    fn is_head_of_session(&self, entry: &Entry) -> bool {
        self.entries
            .iter()
            .find(|e| e.session == entry.session)
            .is_some_and(|first| first.ticket == entry.ticket)
    }

    /// Hand out leases and slots to whoever can run now. Call after any change.
    pub(crate) fn schedule(&mut self) -> Vec<Dispatch> {
        let mut out = Vec::new();
        // Turns that already hold a lease continue first, so they cannot be
        // starved by new work that wants the same folder.
        for index in 0..self.entries.len() {
            if self.entries[index].phase == Phase::Resuming && self.executing() < self.slots {
                let entry = &mut self.entries[index];
                entry.holds_slot = true;
                entry.phase = Phase::Executing;
                out.push(Dispatch {
                    ticket: entry.ticket,
                    session: entry.session,
                    fresh: false,
                });
            }
        }
        for index in 0..self.entries.len() {
            let entry = &self.entries[index];
            if entry.phase != Phase::Queued || !self.is_head_of_session(entry) {
                continue;
            }
            let needs_lease = entry.writes && !entry.holds_lease;
            if needs_lease && self.lease_holder(&entry.workspace, entry.ticket).is_some() {
                continue;
            }
            if self.executing() >= self.slots {
                continue;
            }
            let entry = &mut self.entries[index];
            entry.holds_lease |= entry.writes;
            entry.holds_slot = true;
            entry.started = true;
            entry.phase = Phase::Executing;
            out.push(Dispatch {
                ticket: entry.ticket,
                session: entry.session,
                fresh: true,
            });
        }
        out
    }

    pub(crate) fn phase(&self, ticket: Ticket) -> Option<Phase> {
        self.entry(ticket).map(|e| e.phase)
    }

    fn entry(&self, ticket: Ticket) -> Option<&Entry> {
        self.entries.iter().find(|e| e.ticket == ticket)
    }

    fn entry_mut(&mut self, ticket: Ticket) -> Option<&mut Entry> {
        self.entries.iter_mut().find(|e| e.ticket == ticket)
    }

    /// Why a turn is not running, or `None` when it is (or is unknown).
    pub(crate) fn wait_reason(&self, ticket: Ticket) -> Option<WaitReason> {
        let entry = self.entry(ticket)?;
        match entry.phase {
            Phase::Resuming => Some(WaitReason::ApprovedWaitingForSlot),
            Phase::Queued => {
                if !self.is_head_of_session(entry) {
                    return Some(WaitReason::PreviousMessage);
                }
                if entry.writes
                    && let Some(holder) = self.lease_holder(&entry.workspace, ticket)
                {
                    return Some(WaitReason::Lease {
                        holder: holder.session,
                    });
                }
                Some(WaitReason::NoSlot {
                    executing: self.executing(),
                })
            }
            _ => None,
        }
    }

    /// The agent asked the user something: free the slot, keep the lease.
    pub(crate) fn waiting_for_user(&mut self, ticket: Ticket) {
        if let Some(entry) = self.entry_mut(ticket)
            && matches!(entry.phase, Phase::Executing | Phase::Resuming)
        {
            entry.holds_slot = false;
            entry.phase = Phase::WaitingForUser;
        }
    }

    /// The user answered: the turn needs a slot again before it continues.
    pub(crate) fn user_answered(&mut self, ticket: Ticket) {
        if let Some(entry) = self.entry_mut(ticket)
            && entry.phase == Phase::WaitingForUser
        {
            entry.phase = Phase::Resuming;
        }
    }

    /// Every ending - finished, failed, interrupted, crashed, cancelled - ends here
    /// and releases everything the turn held. Returns whether it existed.
    pub(crate) fn finish(&mut self, ticket: Ticket) -> bool {
        let before = self.entries.len();
        self.entries.retain(|e| e.ticket != ticket);
        self.entries.len() != before
    }

    /// Remove every turn of a session (archive, close-without-keeping, runtime loss).
    pub(crate) fn drop_session(&mut self, session: SessionId) -> Vec<Ticket> {
        let dropped: Vec<Ticket> = self
            .entries
            .iter()
            .filter(|e| e.session == session)
            .map(|e| e.ticket)
            .collect();
        self.entries.retain(|e| e.session != session);
        dropped
    }

    #[cfg(test)]
    pub(crate) fn holds_lease(&self, ticket: Ticket) -> bool {
        self.entry(ticket).is_some_and(|e| e.holds_lease)
    }

    #[cfg(test)]
    pub(crate) fn holds_slot(&self, ticket: Ticket) -> bool {
        self.entry(ticket).is_some_and(|e| e.holds_slot)
    }

    pub(crate) fn is_idle(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> PathBuf {
        PathBuf::from(format!("/work/{name}"))
    }

    fn started(dispatches: &[Dispatch]) -> Vec<Ticket> {
        dispatches.iter().map(|d| d.ticket).collect()
    }

    #[test]
    fn a_session_runs_one_turn_at_a_time_in_order() {
        let mut scheduler = Scheduler::new(4);
        let first = scheduler.submit(SessionId(1), dir("a"), true);
        let second = scheduler.submit(SessionId(1), dir("a"), true);
        assert_eq!(started(&scheduler.schedule()), [first]);
        assert_eq!(
            scheduler.wait_reason(second),
            Some(WaitReason::PreviousMessage)
        );
        scheduler.finish(first);
        assert_eq!(started(&scheduler.schedule()), [second]);
    }

    #[test]
    fn the_global_limit_caps_executing_turns_and_waiters_start_in_arrival_order() {
        let mut scheduler = Scheduler::new(2);
        let t: Vec<Ticket> = (1..=3)
            .map(|n| scheduler.submit(SessionId(n), dir(&n.to_string()), true))
            .collect();
        assert_eq!(started(&scheduler.schedule()), [t[0], t[1]]);
        assert_eq!(
            scheduler.wait_reason(t[2]),
            Some(WaitReason::NoSlot { executing: 2 })
        );
        scheduler.finish(t[0]);
        assert_eq!(started(&scheduler.schedule()), [t[2]]);
    }

    #[test]
    fn one_writer_per_workspace_but_other_folders_and_readers_proceed() {
        let mut scheduler = Scheduler::new(4);
        let writer = scheduler.submit(SessionId(1), dir("repo"), true);
        let blocked = scheduler.submit(SessionId(2), dir("repo"), true);
        let elsewhere = scheduler.submit(SessionId(3), dir("other"), true);
        let reader = scheduler.submit(SessionId(4), dir("repo"), false);
        assert_eq!(started(&scheduler.schedule()), [writer, elsewhere, reader]);
        assert_eq!(
            scheduler.wait_reason(blocked),
            Some(WaitReason::Lease {
                holder: SessionId(1)
            })
        );
        assert!(!scheduler.holds_lease(reader));
        scheduler.finish(writer);
        assert_eq!(started(&scheduler.schedule()), [blocked]);
    }

    #[test]
    fn waiting_for_the_user_frees_the_slot_but_keeps_the_lease() {
        let mut scheduler = Scheduler::new(1);
        let editing = scheduler.submit(SessionId(1), dir("repo"), true);
        let same_folder = scheduler.submit(SessionId(2), dir("repo"), true);
        let other_folder = scheduler.submit(SessionId(3), dir("other"), true);
        assert_eq!(started(&scheduler.schedule()), [editing]);
        scheduler.waiting_for_user(editing);
        assert!(scheduler.holds_lease(editing) && !scheduler.holds_slot(editing));
        // The freed slot goes to a different folder, never to the leased one.
        assert_eq!(started(&scheduler.schedule()), [other_folder]);
        assert_eq!(
            scheduler.wait_reason(same_folder),
            Some(WaitReason::Lease {
                holder: SessionId(1)
            })
        );
    }

    #[test]
    fn answering_needs_a_slot_and_resumers_beat_new_work() {
        let mut scheduler = Scheduler::new(1);
        let editing = scheduler.submit(SessionId(1), dir("repo"), true);
        scheduler.schedule();
        scheduler.waiting_for_user(editing);
        let newcomer = scheduler.submit(SessionId(2), dir("other"), true);
        assert_eq!(started(&scheduler.schedule()), [newcomer]);
        scheduler.user_answered(editing);
        assert_eq!(
            scheduler.wait_reason(editing),
            Some(WaitReason::ApprovedWaitingForSlot)
        );
        assert!(scheduler.schedule().is_empty());
        scheduler.finish(newcomer);
        let resumed = scheduler.schedule();
        assert_eq!(started(&resumed), [editing]);
        assert!(!resumed[0].fresh);
        assert_eq!(scheduler.phase(editing), Some(Phase::Executing));
    }

    #[test]
    fn every_terminal_path_releases_everything_and_dropping_a_session_clears_its_queue() {
        let mut scheduler = Scheduler::new(1);
        let a = scheduler.submit(SessionId(1), dir("repo"), true);
        let queued = scheduler.submit(SessionId(1), dir("repo"), true);
        let b = scheduler.submit(SessionId(2), dir("repo"), true);
        scheduler.schedule();
        scheduler.waiting_for_user(a);
        let dropped = scheduler.drop_session(SessionId(1));
        assert_eq!(dropped, [a, queued]);
        assert!(!scheduler.holds_lease(a));
        assert_eq!(started(&scheduler.schedule()), [b]);
        scheduler.finish(b);
        assert!(scheduler.is_idle());
        assert!(!scheduler.finish(b), "finishing twice is harmless");
    }

    #[test]
    fn a_queued_turn_can_be_cancelled_before_it_starts_without_side_effects() {
        let mut scheduler = Scheduler::new(1);
        let running = scheduler.submit(SessionId(1), dir("repo"), true);
        let waiting = scheduler.submit(SessionId(2), dir("repo"), true);
        scheduler.schedule();
        assert!(scheduler.finish(waiting));
        scheduler.finish(running);
        assert!(scheduler.schedule().is_empty());
        assert!(scheduler.is_idle());
    }

    #[test]
    fn two_turns_never_hold_one_resource_each_while_waiting_for_the_other() {
        // One slot, one folder: the lease is only granted together with the slot,
        // so the blocked turn can never be holding the slot the leaseholder needs.
        let mut scheduler = Scheduler::new(1);
        let holder = scheduler.submit(SessionId(1), dir("repo"), true);
        scheduler.schedule();
        scheduler.waiting_for_user(holder);
        let other = scheduler.submit(SessionId(2), dir("repo"), true);
        assert!(scheduler.schedule().is_empty());
        assert!(!scheduler.holds_slot(other) && !scheduler.holds_lease(other));
        scheduler.user_answered(holder);
        assert_eq!(started(&scheduler.schedule()), [holder]);
    }
}
