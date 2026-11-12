//! Per-port counters and the rates derived from them (ADR 0006
//! components 17 and 21).
//!
//! The switch only keeps good packet and byte counters, as two u32
//! halves, and pushes nothing: a rate is the controller's own
//! difference between two reads. A counter that went **down** is a
//! reset (the switch rebooted, or somebody pressed Clear on the web UI)
//! and never a negative rate, so the interval is dropped rather than
//! reported.
//!
//! The payload goes to `.../switch/counters`, not retained, every
//! `switch.poll.counters_s`.

use alloc::string::String;
use alloc::vec::Vec;

use serde::{Deserialize, Serialize};

use super::model::PortStats;

/// One port's totals plus the rates over the last interval.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PortCounters {
    /// Port number, 1-based.
    pub port: u8,
    /// Good packets sent since the last counter reset.
    pub tx_packets: u64,
    /// Good packets received.
    pub rx_packets: u64,
    /// Good bytes sent.
    pub tx_bytes: u64,
    /// Good bytes received.
    pub rx_bytes: u64,
    /// Packets per second sent over the interval, when there was one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tx_pps: Option<u32>,
    /// Packets per second received.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rx_pps: Option<u32>,
    /// Bits per second sent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tx_bps: Option<u64>,
    /// Bits per second received.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rx_bps: Option<u64>,
}

/// The `.../switch/counters` payload.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortRates {
    /// Payload version, as everywhere else.
    #[serde(default = "one")]
    pub v: u8,
    /// Monotonic milliseconds of the reading.
    pub ts: u64,
    /// Milliseconds the rates are averaged over; 0 when there is no
    /// previous reading to compare with.
    pub interval_ms: u64,
    /// One entry per port the switch reported.
    pub ports: Vec<PortCounters>,
}

const fn one() -> u8 {
    crate::msg::PROTOCOL_VERSION
}

impl PortRates {
    /// Serialise to JSON.
    pub fn to_json(&self) -> Result<String, String> {
        serde_json::to_string(self).map_err(|e| alloc::string::ToString::to_string(&e))
    }
}

/// The rates between two counter reads.
///
/// `previous` is the earlier read with its timestamp; `None` (or a zero
/// interval) yields totals only. A counter that decreased is treated as
/// a reset: that port's rates are left out of this interval.
pub fn rates(
    previous: Option<(&[PortStats], u64)>,
    now: &[PortStats],
    now_ms: u64,
) -> PortRates {
    let (prev, prev_ms) = match previous {
        Some((prev, ms)) if now_ms > ms => (Some(prev), ms),
        _ => (None, 0),
    };
    let interval_ms = prev.map_or(0, |_| now_ms.saturating_sub(prev_ms));
    let mut ports: Vec<PortCounters> = Vec::new();
    for stats in now {
        let mut out = PortCounters {
            port: stats.port,
            tx_packets: stats.tx_packets,
            rx_packets: stats.rx_packets,
            tx_bytes: stats.tx_bytes,
            rx_bytes: stats.rx_bytes,
            ..PortCounters::default()
        };
        if interval_ms > 0
            && let Some(was) = prev.and_then(|p| p.iter().find(|p| p.port == stats.port))
            && was.tx_packets <= stats.tx_packets
            && was.rx_packets <= stats.rx_packets
            && was.tx_bytes <= stats.tx_bytes
            && was.rx_bytes <= stats.rx_bytes
        {
            out.tx_pps = Some(per_second(stats.tx_packets - was.tx_packets, interval_ms) as u32);
            out.rx_pps = Some(per_second(stats.rx_packets - was.rx_packets, interval_ms) as u32);
            out.tx_bps = Some(per_second(
                (stats.tx_bytes - was.tx_bytes).saturating_mul(8),
                interval_ms,
            ));
            out.rx_bps = Some(per_second(
                (stats.rx_bytes - was.rx_bytes).saturating_mul(8),
                interval_ms,
            ));
        }
        ports.push(out);
    }
    PortRates {
        v: one(),
        ts: now_ms,
        interval_ms,
        ports,
    }
}

/// `delta` over `interval_ms`, per second, rounded.
fn per_second(delta: u64, interval_ms: u64) -> u64 {
    if interval_ms == 0 {
        return 0;
    }
    (delta.saturating_mul(1000) + interval_ms / 2) / interval_ms
}

/// True when any counter of any port went down: the switch's counters
/// were reset, which with a lower uptime means it rebooted (ADR 0006
/// component 17).
pub fn counters_went_backwards(previous: &[PortStats], now: &[PortStats]) -> bool {
    now.iter().any(|stats| {
        previous
            .iter()
            .find(|p| p.port == stats.port)
            .is_some_and(|was| {
                was.tx_packets > stats.tx_packets
                    || was.rx_packets > stats.rx_packets
                    || was.tx_bytes > stats.tx_bytes
                    || was.rx_bytes > stats.rx_bytes
            })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn port(port: u8, tx: u64, rx: u64) -> PortStats {
        PortStats {
            port,
            enabled: true,
            link: true,
            tx_packets: tx,
            rx_packets: rx,
            tx_bytes: tx * 100,
            rx_bytes: rx * 100,
        }
    }

    #[test]
    fn the_first_read_has_totals_and_no_rates() {
        let now = alloc::vec![port(9, 1_000, 2_000)];
        let out = rates(None, &now, 10_000);
        assert_eq!(out.interval_ms, 0);
        assert_eq!(out.ports[0].tx_packets, 1_000);
        assert!(out.ports[0].tx_pps.is_none());
    }

    #[test]
    fn rates_are_the_difference_over_the_interval() {
        let was = alloc::vec![port(9, 1_000, 2_000)];
        let now = alloc::vec![port(9, 1_500, 2_000)];
        let out = rates(Some((&was, 10_000)), &now, 20_000);
        assert_eq!(out.interval_ms, 10_000);
        assert_eq!(out.ports[0].tx_pps, Some(50), "500 packets in 10 s");
        assert_eq!(out.ports[0].rx_pps, Some(0));
        assert_eq!(out.ports[0].tx_bps, Some(40_000), "50000 bytes in 10 s");
    }

    #[test]
    fn a_counter_that_went_down_is_a_reset_and_not_a_negative_rate() {
        let was = alloc::vec![port(9, 5_000, 5_000)];
        let now = alloc::vec![port(9, 10, 20)];
        assert!(counters_went_backwards(&was, &now));
        let out = rates(Some((&was, 10_000)), &now, 20_000);
        assert_eq!(out.ports[0].tx_packets, 10, "the totals are still reported");
        assert!(out.ports[0].tx_pps.is_none(), "but no rate for this interval");

        let forward = alloc::vec![port(9, 5_001, 5_001)];
        assert!(!counters_went_backwards(&was, &forward));
    }

    #[test]
    fn the_payload_carries_the_version() {
        let out = rates(None, &alloc::vec![port(1, 0, 0)], 1);
        let json = out.to_json().expect("serialises");
        assert!(json.starts_with("{\"v\":1,"), "{json}");
        assert!(!json.contains("tx_pps"), "no rate on the first read");
    }
}
