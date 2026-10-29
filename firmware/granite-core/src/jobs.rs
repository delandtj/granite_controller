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

/// A command that is waiting for its plane job.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingJob {
    /// The plane job id.
    pub job: JobId,
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
}

/// A job that ended, with everything the transports have to publish.
#[derive(Debug, Clone, PartialEq)]
pub struct JobDone {
    /// The command that asked for it, when the log still had it.
    pub pending: Option<PendingJob>,
    /// The ack to publish on `.../ack/<id>` or to return from
    /// `?wait=1`.
    pub reply: Reply,
    /// The event to publish on `.../event`.
    pub event: EventKind,
    /// How it ended.
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

    /// Record a submitted job.
    pub fn note(
        &mut self,
        job: JobId,
        command_id: &str,
        verb: CommandKind,
        target: Target,
        nodes: &[NodeId],
        now_ms: u64,
    ) {
        while self.pending.len() >= MAX_PENDING {
            self.pending.pop_front();
        }
        self.pending.push_back(PendingJob {
            job,
            command_id: String::from(command_id),
            verb,
            target,
            nodes: nodes.to_vec(),
            submitted_ms: now_ms,
        });
        for node in nodes {
            self.note_last(
                *node,
                LastJob {
                    id: String::from(command_id),
                    job,
                    verb: String::from(verb.as_str()),
                    started_ms: now_ms,
                    finished_ms: None,
                    result: None,
                },
            );
        }
    }

    /// Take the pending entry of a job, if there is one.
    pub fn take(&mut self, job: JobId) -> Option<PendingJob> {
        let at = self.pending.iter().position(|p| p.job == job)?;
        self.pending.remove(at)
    }

    /// The pending entry of a job.
    pub fn pending(&self, job: JobId) -> Option<&PendingJob> {
        self.pending.iter().find(|p| p.job == job)
    }

    /// The job a command id is waiting for, if it is waiting for one.
    pub fn job_of(&self, command_id: &str) -> Option<JobId> {
        self.pending
            .iter()
            .find(|p| p.command_id == command_id)
            .map(|p| p.job)
    }

    /// How many jobs are waiting.
    pub fn len(&self) -> usize {
        self.pending.len()
    }

    /// True when nothing is waiting.
    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }

    /// Fold a finished job into the log: the ack, the event and the
    /// per-node record.
    pub fn finish(&mut self, finished: &FinishedJob, now_ms: u64) -> JobDone {
        let pending = self.take(finished.id);
        let verb = pending.as_ref().map(|p| p.verb).or_else(|| {
            // A job the log has forgotten (or one the client submitted
            // itself, like the intended-OFF re-apply): the command's own
            // verb name still identifies it.
            CommandKind::from_str_opt(finished.command.verb())
        });
        let command_id = pending
            .as_ref()
            .map(|p| p.command_id.clone())
            .unwrap_or_else(|| alloc::format!("job-{}", finished.id));
        let detail = finished.code.map(result_code_name);
        let reply = Reply::of_outcome(&command_id, finished.outcome, detail).with_job(finished.id);
        let nodes: Vec<NodeId> = match pending.as_ref() {
            Some(p) if !p.nodes.is_empty() => p.nodes.clone(),
            _ => finished.command.nodes(),
        };
        let result = match detail {
            Some(code) if !finished.outcome.is_ok() => {
                alloc::format!("{}: {code}", finished.outcome.as_str())
            }
            _ => String::from(finished.outcome.as_str()),
        };
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
        let action = verb.unwrap_or(CommandKind::On);
        let event = if finished.outcome.is_ok() {
            EventKind::ActionDone {
                id: command_id.clone(),
                node: nodes.first().copied(),
                action,
                result: result.clone(),
            }
        } else {
            EventKind::ActionFailed {
                id: command_id.clone(),
                node: nodes.first().copied(),
                action,
                error: result.clone(),
            }
        };
        JobDone {
            pending,
            reply,
            event,
            outcome: finished.outcome,
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
        FinishedJob {
            id,
            command: PlaneJobCommand::Node(NodeCommand::on(3)),
            outcome,
            code: None,
            submitted_ms: 100,
            finished_ms: 500,
        }
    }

    #[test]
    fn an_ack_finds_its_command_again() {
        let mut log = JobLog::new();
        log.note(7, "c-1", CommandKind::On, Target::Node(3), &[3], 100);
        assert_eq!(log.job_of("c-1"), Some(7));
        assert_eq!(log.len(), 1);
        assert_eq!(log.last(3).and_then(|r| r.result.clone()), None);

        let done = log.finish(&finished(7, JobOutcome::Done), 500);
        assert_eq!(done.reply.id, "c-1");
        assert_eq!(done.reply.job, Some(7));
        assert!(done.reply.ok);
        assert!(log.is_empty());
        let record = log.last(3).expect("a last job");
        assert_eq!(record.finished_ms, Some(500));
        assert_eq!(record.result.as_deref(), Some("done"));
        assert!(matches!(done.event, EventKind::ActionDone { .. }));
    }

    #[test]
    fn a_job_nobody_waits_for_still_finishes() {
        let mut log = JobLog::new();
        let done = log.finish(&finished(9, JobOutcome::Absent), 500);
        assert!(done.pending.is_none());
        assert_eq!(done.reply.id, "job-9");
        assert!(!done.reply.ok);
        assert!(matches!(done.event, EventKind::ActionFailed { .. }));
        // The command itself names the node, so the record is still
        // written.
        assert!(log.last(3).is_some());
    }

    #[test]
    fn the_log_is_bounded() {
        let mut log = JobLog::new();
        for job in 0..(MAX_PENDING as JobId + 10) {
            log.note(job + 1, "c", CommandKind::On, Target::Node(1), &[1], 0);
        }
        assert_eq!(log.len(), MAX_PENDING);
        assert!(log.pending(1).is_none(), "the oldest went first");
    }
}
