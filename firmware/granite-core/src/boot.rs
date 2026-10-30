//! When the per-node boot policies run.
//!
//! ADR 0004 decision 2 gives every node a boot policy, and the ADR's
//! expert answer says when it applies:
//!
//! > **Q: The controller reboots. Does any node change state?**
//! > A: No. [...] the controller rediscovers the planes, reads their
//! > state, and applies boot policy only to nodes whose policy says so
//! > and only on a plane boot (boot count changed), never on its own
//! > boot.
//!
//! So the trigger is a *plane* boot, and this module is the thing that
//! recognises one. [`BootPolicyWatch::due`] is called with every
//! snapshot and gives back the nodes to command:
//!
//! - **A plane the controller is already watching reboots.** Its boot
//!   count moves, or it raises the "intended state lost" flag of a core
//!   power loss (after which the count comes back at 1 and may not
//!   differ). Either edge is a plane boot.
//! - **A plane is seen for the first time since this controller
//!   started.** The controller cannot tell a plane that has been up for
//!   a week from one it has just met, so by itself this is *not* a
//!   trigger: that is exactly the "never on its own boot" case. The one
//!   exception is a plane that says it lost its intended state *and*
//!   has just booted ([`BOOT_RECENT_S`]), which is a site power loss
//!   that took the controller with it - the case boot policy `on`
//!   exists for.
//!
//! Each trigger is answered once, [`BOOT_POLICY_SETTLE_MS`] after it is
//! seen, which gives the plane's slot reports time to arrive so the
//! nodes of that plane are known.
//!
//! The watch keeps nothing across a controller restart on purpose: a
//! fresh controller has seen no plane boot. What survives a restart is
//! the intended-OFF set ([`crate::plane::intent`]), which is a
//! different mechanism with a different job.

use alloc::vec::Vec;

use crate::NodeId;
use crate::config::NodesCfg;
use crate::node::BootPolicy;
use crate::observed::Observed;

/// How long after a plane boot the policies are applied \[ms\].
///
/// Long enough for the plane's STATUS, SUMMARY and slot reports, so
/// every node of that plane is in the snapshot when its policy is read.
pub const BOOT_POLICY_SETTLE_MS: u64 = 3_000;

/// How young a plane's uptime must be for a first sighting with the
/// "intended state lost" flag to count as a plane boot \[s\].
///
/// A plane that lost its power minutes or hours ago and has been
/// running ever since is not a boot the controller has just missed; it
/// is a flag that stays up until a reset which keeps VDD (ADR 0002
/// decision 3). Without this, every controller reboot would re-run the
/// policies for as long as that flag is set.
pub const BOOT_RECENT_S: u32 = 60;

/// What the watch remembers about one plane.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Seen {
    boot_count: u32,
    /// The plane said its intended state was lost.
    lost: bool,
    /// A boot is waiting to be answered at this time \[ms\].
    due_at: Option<u64>,
}

/// Recognises plane boots and says which nodes they move.
#[derive(Debug, Clone, Default)]
pub struct BootPolicyWatch {
    seen: [Option<Seen>; crate::MAX_PLANES],
}

impl BootPolicyWatch {
    /// A watch that has seen nothing.
    pub fn new() -> Self {
        Self::default()
    }

    /// The nodes whose boot policy is due now, with the policy to run.
    ///
    /// Call it on every tick with the current snapshot. It returns
    /// something only on the tick that answers a plane boot, and each
    /// boot is answered once.
    pub fn due(
        &mut self,
        now_ms: u64,
        observed: &Observed,
        nodes: &NodesCfg,
    ) -> Vec<(NodeId, BootPolicy)> {
        let mut out: Vec<(NodeId, BootPolicy)> = Vec::new();
        for plane in &observed.planes {
            let index = plane.plane as usize;
            if index >= crate::MAX_PLANES || plane.info_ms == 0 {
                continue;
            }
            let before = self.seen[index];
            let booted = match before {
                // First sight since this controller started: only a
                // plane that has just lost its own power counts.
                None => plane.intended_state_lost && plane.uptime_s <= BOOT_RECENT_S,
                // A plane the controller was watching: the boot count
                // moved, or the power-loss flag went up.
                Some(prev) => {
                    prev.boot_count != plane.boot_count || (!prev.lost && plane.intended_state_lost)
                }
            };
            let mut due_at = before.and_then(|prev| prev.due_at);
            if booted {
                due_at = Some(now_ms.saturating_add(BOOT_POLICY_SETTLE_MS));
            }
            // The slot reports need to be in before the policies are
            // read, which is what the settle time is for.
            let run = due_at.is_some_and(|at| now_ms >= at);
            self.seen[index] = Some(Seen {
                boot_count: plane.boot_count,
                lost: plane.intended_state_lost,
                due_at: if run { None } else { due_at },
            });
            if !run {
                continue;
            }
            for node in observed.nodes.iter().filter(|n| n.plane == plane.plane) {
                if !node.present {
                    continue;
                }
                let policy = nodes.boot_policy(node.node);
                if policy != BootPolicy::Leave {
                    out.push((node.node, policy));
                }
            }
        }
        out.sort_unstable_by_key(|(node, _)| *node);
        out
    }

    /// True while a plane boot is waiting for its settle time.
    pub fn is_pending(&self) -> bool {
        self.seen
            .iter()
            .any(|s| s.is_some_and(|seen| seen.due_at.is_some()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::node::NodeSettings;
    use crate::observed::{NodeObs, PlaneObs};
    use crate::plane::Plane;
    use crate::plane::model::{HostState, PowerState};

    /// A snapshot with one plane at `plane` and its two nodes present.
    fn snapshot(plane: u8, boot_count: u32, lost: bool, uptime_s: u32) -> Observed {
        let mut o = Observed::new();
        let address = granite_proto::ADDR_BASE + plane;
        let mut obs = PlaneObs::of_plane(&Plane::new(address));
        obs.info_ms = 10;
        obs.ts_ms = 20;
        obs.boot_count = boot_count;
        obs.intended_state_lost = lost;
        obs.uptime_s = uptime_s;
        o.planes.push(obs);
        for slot in 0..2u8 {
            let id = crate::plane::node_id(address, slot).expect("a node id");
            let mut node = NodeObs::unknown(id, address, slot);
            node.present = true;
            node.power = PowerState::On;
            node.host = HostState::On;
            o.nodes.push(node);
        }
        o
    }

    fn policies() -> NodesCfg {
        let mut cfg = NodesCfg::default();
        cfg.nodes.push(NodeSettings {
            node: 1,
            boot_policy: BootPolicy::On,
            ..NodeSettings::for_node(1)
        });
        cfg.nodes.push(NodeSettings {
            node: 2,
            boot_policy: BootPolicy::Off,
            ..NodeSettings::for_node(2)
        });
        cfg
    }

    #[test]
    fn a_controller_start_alone_runs_nothing() {
        let mut watch = BootPolicyWatch::new();
        let cfg = policies();
        // A plane that has been up for a day: the controller has just
        // met it, and that is not a plane boot.
        let o = snapshot(0, 7, false, 86_400);
        for now in [0u64, 1_000, 3_000, 10_000, 60_000] {
            assert!(
                watch.due(now, &o, &cfg).is_empty(),
                "a controller reboot moves no node (t = {now})"
            );
        }
        assert!(!watch.is_pending());
    }

    #[test]
    fn a_plane_reboot_runs_the_policies_once() {
        let mut watch = BootPolicyWatch::new();
        let cfg = policies();
        let before = snapshot(0, 7, false, 86_400);
        assert!(watch.due(0, &before, &cfg).is_empty());

        // The plane reboots: its boot count moves.
        let after = snapshot(0, 8, false, 1);
        assert!(
            watch.due(1_000, &after, &cfg).is_empty(),
            "the slot reports are given their settle time"
        );
        assert!(watch.is_pending());
        assert_eq!(
            watch.due(1_000 + BOOT_POLICY_SETTLE_MS, &after, &cfg),
            alloc::vec![(1, BootPolicy::On), (2, BootPolicy::Off)]
        );
        assert!(!watch.is_pending());
        // And once only.
        assert!(watch.due(20_000, &after, &cfg).is_empty());
        assert!(watch.due(60_000, &after, &cfg).is_empty());
    }

    #[test]
    fn a_plane_power_loss_runs_them_even_at_the_first_sight() {
        let mut watch = BootPolicyWatch::new();
        let cfg = policies();
        // Site power came back: the controller and the plane booted
        // together, and the plane says its intended state is gone.
        let fresh = snapshot(0, 1, true, 2);
        assert!(watch.due(0, &fresh, &cfg).is_empty(), "settling");
        assert_eq!(
            watch.due(BOOT_POLICY_SETTLE_MS, &fresh, &cfg),
            alloc::vec![(1, BootPolicy::On), (2, BootPolicy::Off)]
        );
        // The flag stays up, but the boot is answered.
        assert!(watch.due(60_000, &fresh, &cfg).is_empty());

        // A controller that meets the same plane an hour later does not
        // run them again: the flag is still up, the uptime is not.
        let mut later = BootPolicyWatch::new();
        let old = snapshot(0, 1, true, 3_600);
        assert!(later.due(0, &old, &cfg).is_empty());
        assert!(later.due(BOOT_POLICY_SETTLE_MS, &old, &cfg).is_empty());
    }

    #[test]
    fn a_power_loss_flag_that_goes_up_while_watching_is_a_boot() {
        let mut watch = BootPolicyWatch::new();
        let cfg = policies();
        // The boot count is 1 before and after: a core power loss takes
        // the counter with it, so only the flag says what happened.
        let before = snapshot(0, 1, false, 500);
        assert!(watch.due(0, &before, &cfg).is_empty());
        let after = snapshot(0, 1, true, 3);
        assert!(watch.due(1_000, &after, &cfg).is_empty());
        assert_eq!(
            watch.due(1_000 + BOOT_POLICY_SETTLE_MS, &after, &cfg).len(),
            2
        );
    }

    #[test]
    fn only_the_nodes_of_the_plane_that_booted_move() {
        let mut watch = BootPolicyWatch::new();
        let mut cfg = policies();
        // Node 9 is slot 0 of plane 1.
        cfg.nodes.push(NodeSettings {
            node: 9,
            boot_policy: BootPolicy::On,
            ..NodeSettings::for_node(9)
        });
        let mut both = snapshot(0, 7, false, 86_400);
        let plane1 = snapshot(1, 7, false, 86_400);
        both.planes.extend(plane1.planes);
        both.nodes.extend(plane1.nodes);
        assert!(watch.due(0, &both, &cfg).is_empty());

        // Plane 1 reboots; plane 0 does not.
        let mut after = snapshot(0, 7, false, 86_401);
        let rebooted = snapshot(1, 8, false, 1);
        after.planes.extend(rebooted.planes);
        after.nodes.extend(rebooted.nodes);
        assert!(watch.due(1_000, &after, &cfg).is_empty());
        assert_eq!(
            watch.due(1_000 + BOOT_POLICY_SETTLE_MS, &after, &cfg),
            alloc::vec![(9, BootPolicy::On)],
            "node 1 and 2 are on the plane that kept running"
        );
    }

    #[test]
    fn a_node_that_says_leave_is_never_in_the_list() {
        let mut watch = BootPolicyWatch::new();
        let cfg = NodesCfg::default();
        let before = snapshot(0, 7, false, 86_400);
        assert!(watch.due(0, &before, &cfg).is_empty());
        let after = snapshot(0, 8, false, 1);
        watch.due(1_000, &after, &cfg);
        assert!(
            watch
                .due(1_000 + BOOT_POLICY_SETTLE_MS, &after, &cfg)
                .is_empty(),
            "every node defaults to leave"
        );
    }
}
