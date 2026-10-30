//! The plane jobs a command produced, and the acks that follow them.
//!
//! A command does not finish when the dispatcher returns: it becomes a
//! plane job and the reply says `accepted` (ADR 0004 decision 6). The
//! plane reports the result some time later, and the MQTT ack and the
//! HTTP job result have to find the command id again. That bookkeeping
//! is the same on the board and in the simulator, so it lives here
//! rather than twice in the two transports:
//!
//! - [`JobLog::note`] when [`crate::dispatch::dispatch`] submits a job.
//! - [`JobLog::finish`] for every [`FinishedJob`] the plane client
//!   drains: it gives back the ack to publish and the event to log.
//! - [`JobLog::apply`] copies the last job of each node into the
//!   snapshot, which is what a dashboard polls.
//!
//! One command can become several plane jobs: a node verb on target
//! `all` is one job per present node, the way the same verb on one node
//! would be (ADR 0004 decision 4, "the rev C verbs keep their names and
//! their meaning"). The log keeps one entry per *command*, so the ack
//! goes out once, when the last of its jobs has reported, and carries
//! the worst outcome of the group. Each node's own job still writes that
//! node's `last_job` record.

use alloc::collections::BTreeMap;
use alloc::collections::VecDeque;
use alloc::string::String;
use alloc::vec::Vec;

use crate::msg::{CommandKind, EventKind, Reply};
use crate::node::LastJob;
use crate::observed::Observed;
use crate::plane::{FinishedJob, JobId, JobOutcome};
use crate::{NodeId, Target};

/// Jobs the log remembers at once.
///
/// One per node is the worst an operator can produce by hand; the limit
/// is there so a client that submits and never reads cannot grow the
/// log without bound. The oldest entry is dropped first, and a job whose
/// entry is gone still finishes - it just finishes without an ack.
pub const MAX_PENDING: usize = crate::MAX_NODES;

/// Last jobs the log keeps per node. One per node id.
pub const MAX_LAST: usize = crate::MAX_NODES;

/// One plane job a command became, as [`crate::dispatch::dispatch`]
/// submitted it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubmittedJob {
    /// The plane job id.
    pub job: JobId,
    /// The nodes that job acts on: one for a node verb, the whole set
    /// for `on_all`, none for a plane or unit verb.
    pub nodes: Vec<NodeId>,
}

impl SubmittedJob {
    /// One job over one set of nodes.
    pub fn new(job: JobId, nodes: Vec<NodeId>) -> Self {
        SubmittedJob { job, nodes }
    }
}

/// A command that is waiting for its plane jobs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingJob {
    /// The plane jobs it became, in submission order. One for a single
    /// node, one per present node for a verb on `all`.
    pub jobs: Vec<SubmittedJob>,
    /// The command id the client chose.
    pub command_id: String,
    /// The verb.
    pub verb: CommandKind,
    /// What it acts on.
    pub target: Target,
    /// The nodes it acts on, as the dispatcher resolved them.
    pub nodes: Vec<NodeId>,
    /// When it was accepted \[ms\].
    pub submitted_ms: u64,
    /// The jobs that have reported, with how they ended.
    pub finished: Vec<(JobId, JobOutcome)>,
}

impl PendingJob {
    /// The job id the acks carry: the first of the group.
    pub fn job(&self) -> JobId {
        self.jobs.first().map_or(0, |j| j.job)
    }

    /// True when every job of this command has reported.
    pub fn is_complete(&self) -> bool {
        self.finished.len() >= self.jobs.len()
    }

    /// True when this job id belongs to the command.
    pub fn holds(&self, job: JobId) -> bool {
        self.jobs.iter().any(|j| j.job == job)
    }
}

/// A job that ended, with everything the transports have to publish.
#[derive(Debug, Clone, PartialEq)]
pub struct JobDone {
    /// The command that asked for it, once every job of it has reported
    /// (and `None` for a job the log no longer remembers).
    pub pending: Option<PendingJob>,
    /// The ack to publish on `.../ack/<id>` or to return from `?wait=1`.
    /// `None` while other jobs of the same command are still running.
    pub reply: Option<Reply>,
    /// The event to publish on `.../event`, on the same condition as
    /// the ack.
    pub event: Option<EventKind>,
    /// How this one job ended.
    pub outcome: JobOutcome,
}

/// The pending jobs and the last job of every node.
#[derive(Debug, Clone, Default)]
pub struct JobLog {
    pending: VecDeque<PendingJob>,
    last: BTreeMap<NodeId, LastJob>,
}

impl JobLog {
    /// An empty log.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record the jobs one command was submitted as.
    pub fn note(
        &mut self,
        command_id: &str,
        verb: CommandKind,
        target: Target,
        jobs: &[SubmittedJob],
        now_ms: u64,
    ) {
        while self.pending.len() >= MAX_PENDING {
            self.pending.pop_front();
        }
        let mut nodes: Vec<NodeId> = Vec::new();
        for submitted in jobs {
            for node in &submitted.nodes {
                if !nodes.contains(node) {
                    nodes.push(*node);
                }
                self.note_last(
                    *node,
                    LastJob {
                        id: String::from(command_id),
                        job: submitted.job,
                        verb: String::from(verb.as_str()),
                        started_ms: now_ms,
                        finished_ms: None,
                        result: None,
                    },
                );
            }
        }
        nodes.sort_unstable();
        self.pending.push_back(PendingJob {
            jobs: jobs.to_vec(),
            command_id: String::from(command_id),
            verb,
            target,
            nodes,
            submitted_ms: now_ms,
            finished: Vec::new(),
        });
    }

    /// Take the pending entry that holds a job, if there is one.
    pub fn take(&mut self, job: JobId) -> Option<PendingJob> {
        let at = self.pending.iter().position(|p| p.holds(job))?;
        self.pending.remove(at)
    }

    /// The pending entry that holds a job.
    pub fn pending(&self, job: JobId) -> Option<&PendingJob> {
        self.pending.iter().find(|p| p.holds(job))
    }

    /// The first job a command id is waiting for, if it is waiting for
    /// one: the id its acks carry.
    pub fn job_of(&self, command_id: &str) -> Option<JobId> {
        self.pending
            .iter()
            .find(|p| p.command_id == command_id)
            .map(PendingJob::job)
    }

    /// How many jobs are waiting.
    pub fn len(&self) -> usize {
        self.pending.len()
    }

    /// True when nothing is waiting.
    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }

    /// Fold a finished job into the log: the per-node record always, and
    /// the ack and the event once every job of its command has reported.
    pub fn finish(&mut self, finished: &FinishedJob, now_ms: u64) -> JobDone {
        // The entry stays in the log while other jobs of the same
        // command are still running; only the last one takes it out.
        let at = self.pending.iter().position(|p| p.holds(finished.id));
        if let Some(at) = at {
            self.pending[at]
                .finished
                .push((finished.id, finished.outcome));
        }
        // Copied out of the entry: the per-node records below need the
        // log mutably.
        let entry: Option<(String, CommandKind, Vec<NodeId>, bool)> = at.map(|at| {
            let p = &self.pending[at];
            (
                p.command_id.clone(),
                p.verb,
                p.nodes.clone(),
                p.is_complete(),
            )
        });
        let verb = entry.as_ref().map(|e| e.1).or_else(|| {
            // A job the log has forgotten (or one the client submitted
            // itself, like the intended-OFF re-apply): the command's own
            // verb name still identifies it.
            CommandKind::from_str_opt(finished.command.verb())
        });
        let command_id = entry
            .as_ref()
            .map(|e| e.0.clone())
            .unwrap_or_else(|| alloc::format!("job-{}", finished.id));
        let detail = finished.code.map(result_code_name);
        let result = match detail {
            Some(code) if !finished.outcome.is_ok() => {
                alloc::format!("{}: {code}", finished.outcome.as_str())
            }
            _ => String::from(finished.outcome.as_str()),
        };
        // This job's own nodes, so a fan-out writes one record per node
        // as each of its jobs reports.
        let mut nodes: Vec<NodeId> = finished.command.nodes();
        if nodes.is_empty() {
            nodes = entry.as_ref().map(|e| e.2.clone()).unwrap_or_default();
        }
        for node in &nodes {
            let mut record = self.last.get(node).cloned().unwrap_or(LastJob {
                id: command_id.clone(),
                job: finished.id,
                verb: String::from(finished.command.verb()),
                started_ms: finished.submitted_ms,
                finished_ms: None,
                result: None,
            });
            record.job = finished.id;
            record.finished_ms = Some(now_ms);
            record.result = Some(result.clone());
            self.note_last(*node, record);
        }
        let complete = entry.as_ref().is_none_or(|e| e.3);
        if !complete {
            return JobDone {
                pending: None,
                reply: None,
                event: None,
                outcome: finished.outcome,
            };
        }
        let pending = at.and_then(|at| self.pending.remove(at));
        let action = verb.unwrap_or(CommandKind::On);
        // One ack per command: the worst outcome of its jobs, and the
        // node only when the command named exactly one.
        let (outcome, failures, total) = group_outcome(pending.as_ref(), finished);
        let node = match pending.as_ref() {
            Some(p) if p.jobs.len() > 1 => None,
            _ => nodes.first().copied(),
        };
        let text = if total > 1 && failures > 0 {
            alloc::format!("{failures} of {total} failed: {}", outcome.as_str())
        } else if total > 1 {
            String::from(outcome.as_str())
        } else {
            result.clone()
        };
        let job = pending.as_ref().map_or(finished.id, PendingJob::job);
        let reply = if outcome.is_ok() {
            Reply::ok(&command_id, text.clone()).with_job(job)
        } else {
            Reply::err(&command_id, text.clone()).with_job(job)
        };
        let event = if outcome.is_ok() {
            EventKind::ActionDone {
                id: command_id.clone(),
                node,
                action,
                result: text,
            }
        } else {
            EventKind::ActionFailed {
                id: command_id.clone(),
                node,
                action,
                error: text,
            }
        };
        JobDone {
            pending,
            reply: Some(reply),
            event: Some(event),
            outcome,
        }
    }

    /// The last job of one node.
    pub fn last(&self, node: NodeId) -> Option<&LastJob> {
        self.last.get(&node)
    }

    /// Copy the per-node records into a snapshot.
    pub fn apply(&self, observed: &mut Observed) {
        for node in observed.nodes.iter_mut() {
            if let Some(record) = self.last.get(&node.node) {
                node.last_job = Some(record.clone());
            }
        }
    }

    fn note_last(&mut self, node: NodeId, record: LastJob) {
        if self.last.len() >= MAX_LAST
            && !self.last.contains_key(&node)
            && let Some(oldest) = self
                .last
                .iter()
                .min_by_key(|(_, r)| r.started_ms)
                .map(|(id, _)| *id)
        {
            self.last.remove(&oldest);
        }
        self.last.insert(node, record);
    }
}

/// What one ack reports for a command's jobs: the worst outcome, how
/// many of them failed and how many there were.
///
/// A command that became one job reports that job, exactly as before the
/// fan-out existed. A command over `all` reports the group.
fn group_outcome(
    pending: Option<&PendingJob>,
    finished: &FinishedJob,
) -> (JobOutcome, usize, usize) {
    let Some(p) = pending else {
        return (finished.outcome, usize::from(!finished.outcome.is_ok()), 1);
    };
    let total = p.jobs.len().max(1);
    let failures = p.finished.iter().filter(|(_, o)| !o.is_ok()).count();
    let worst = p
        .finished
        .iter()
        .map(|(_, o)| *o)
        .max_by_key(|o| outcome_rank(*o))
        .unwrap_or(finished.outcome);
    (worst, failures, total)
}

/// How bad an outcome is, so a group of jobs can be answered with one:
/// a timeout says least about what happened, a success most.
const fn outcome_rank(outcome: JobOutcome) -> u8 {
    match outcome {
        JobOutcome::Done => 0,
        JobOutcome::Absent => 1,
        JobOutcome::Failed => 2,
        JobOutcome::LinkLost => 3,
        JobOutcome::TimedOut => 4,
    }
}

/// The lowercase name of a plane result code, for an ack's error text.
pub const fn result_code_name(code: granite_proto::ResultCode) -> &'static str {
    use granite_proto::ResultCode as R;
    match code {
        R::Ok => "ok",
        R::Accepted => "accepted",
        R::Busy => "busy",
        R::BadArgument => "bad_argument",
        R::Conflict => "conflict",
        R::NotPresent => "not_present",
        R::UnknownTag => "unknown_tag",
        R::Refused => "refused",
        R::Failed => "failed",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plane::{Command as PlaneJobCommand, NodeCommand};

    fn finished(id: JobId, outcome: JobOutcome) -> FinishedJob {
        finished_on(id, 3, outcome)
    }

    fn finished_on(id: JobId, node: NodeId, outcome: JobOutcome) -> FinishedJob {
        FinishedJob {
            id,
            command: PlaneJobCommand::Node(NodeCommand::on(node)),
            outcome,
            code: None,
            submitted_ms: 100,
            finished_ms: 500,
        }
    }

    fn one(job: JobId, node: NodeId) -> Vec<SubmittedJob> {
        alloc::vec![SubmittedJob::new(job, alloc::vec![node])]
    }

    #[test]
    fn an_ack_finds_its_command_again() {
        let mut log = JobLog::new();
        log.note("c-1", CommandKind::On, Target::Node(3), &one(7, 3), 100);
        assert_eq!(log.job_of("c-1"), Some(7));
        assert_eq!(log.len(), 1);
        assert_eq!(log.last(3).and_then(|r| r.result.clone()), None);

        let done = log.finish(&finished(7, JobOutcome::Done), 500);
        let reply = done.reply.expect("the command is finished");
        assert_eq!(reply.id, "c-1");
        assert_eq!(reply.job, Some(7));
        assert!(reply.ok);
        assert!(log.is_empty());
        let record = log.last(3).expect("a last job");
        assert_eq!(record.finished_ms, Some(500));
        assert_eq!(record.result.as_deref(), Some("done"));
        assert!(matches!(done.event, Some(EventKind::ActionDone { .. })));
    }

    #[test]
    fn a_job_nobody_waits_for_still_finishes() {
        let mut log = JobLog::new();
        let done = log.finish(&finished(9, JobOutcome::Absent), 500);
        assert!(done.pending.is_none());
        let reply = done.reply.expect("an ack all the same");
        assert_eq!(reply.id, "job-9");
        assert!(!reply.ok);
        assert!(matches!(done.event, Some(EventKind::ActionFailed { .. })));
        // The command itself names the node, so the record is still
        // written.
        assert!(log.last(3).is_some());
    }

    #[test]
    fn a_fan_out_is_one_entry_and_one_ack() {
        let mut log = JobLog::new();
        let jobs = alloc::vec![
            SubmittedJob::new(1, alloc::vec![1]),
            SubmittedJob::new(2, alloc::vec![2]),
            SubmittedJob::new(3, alloc::vec![3]),
        ];
        log.note("c-all", CommandKind::ForceOff, Target::All, &jobs, 100);
        assert_eq!(log.len(), 1, "one entry per command, not per job");
        assert_eq!(log.job_of("c-all"), Some(1));
        assert!(log.pending(3).is_some(), "every job finds the entry");

        // The first two report: no ack yet, but their nodes have their
        // records.
        let first = log.finish(&finished_on(1, 1, JobOutcome::Done), 200);
        assert!(first.reply.is_none() && first.event.is_none());
        assert_eq!(log.last(1).and_then(|r| r.result.clone()).as_deref(), Some("done"));
        assert!(
            log.finish(&finished_on(2, 2, JobOutcome::Failed), 300)
                .reply
                .is_none()
        );
        assert_eq!(log.len(), 1, "the entry waits for the last job");

        // The last one closes the command.
        let last = log.finish(&finished_on(3, 3, JobOutcome::Done), 400);
        let reply = last.reply.expect("the group is finished");
        assert_eq!(reply.id, "c-all");
        assert_eq!(reply.job, Some(1), "the group's first job id");
        assert!(!reply.ok, "one of the three failed");
        assert_eq!(reply.error.as_deref(), Some("1 of 3 failed: failed"));
        assert_eq!(last.outcome, JobOutcome::Failed);
        assert!(log.is_empty());
        match last.event {
            Some(EventKind::ActionFailed { node, .. }) => {
                assert_eq!(node, None, "a group names no single node")
            }
            other => panic!("expected action_failed, got {other:?}"),
        }
    }

    #[test]
    fn the_log_is_bounded() {
        let mut log = JobLog::new();
        for job in 0..(MAX_PENDING as JobId + 10) {
            log.note("c", CommandKind::On, Target::Node(1), &one(job + 1, 1), 0);
        }
        assert_eq!(log.len(), MAX_PENDING);
        assert!(log.pending(1).is_none(), "the oldest went first");
    }
}
