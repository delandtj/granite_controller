//! The HAL surface: error formatting, ROM id helpers, and a compile-level
//! proof that every trait is implementable on the host with plain
//! synchronous code (no async, no async_trait).

use std::collections::BTreeMap;

use granite_core::hal::{
    BoardTemp, BootReason, BusVoltage, Clock, DryInputs, HalError, HalResult, LedPattern,
    NodeSense, NodeSwitches, Persist, Probes, Reboot, RomId, StatusLed, Switch, TEMP_MISSING,
    rom_id_from_hex, rom_id_hex,
};

#[test]
fn rom_ids_round_trip_through_hex() {
    for rom in [0u64, 1, 0x28_1234_5678_9abc, u64::MAX] {
        let hex = rom_id_hex(rom);
        assert_eq!(hex.len(), 16, "{hex}");
        assert_eq!(rom_id_from_hex(&hex), Some(rom));
    }
    assert_eq!(
        rom_id_from_hex("0x28000000000001"),
        Some(0x28_0000_0000_0001)
    );
    assert_eq!(
        rom_id_from_hex(" 2800000000000001 "),
        Some(0x2800_0000_0000_0001)
    );
    assert_eq!(rom_id_from_hex(""), None);
    assert_eq!(rom_id_from_hex("zz"), None);
    assert_eq!(rom_id_from_hex("00000000000000000"), None);
}

#[test]
fn errors_and_enums_have_stable_text() {
    assert_eq!(HalError::ExpanderFault.to_string(), "expander fault");
    assert_eq!(HalError::Bus.to_string(), "bus error");
    assert_eq!(
        HalError::Other("i2c nack at 0x20".into()).to_string(),
        "i2c nack at 0x20"
    );
    assert_eq!(Switch::Pwr.to_string(), "pwr");
    assert_eq!(Switch::Rst.to_string(), "rst");
    assert_eq!(BootReason::TaskWatchdog.to_string(), "task_watchdog");
    assert_eq!(
        serde_json::to_string(&LedPattern::OtaPending).unwrap(),
        "\"ota_pending\""
    );
    assert_eq!(
        serde_json::to_string(&BootReason::BrownOut).unwrap(),
        "\"brown_out\""
    );
    assert_eq!(TEMP_MISSING as u16, 0x8000);
}

/// One struct implementing every trait, the way granite-fw will.
#[derive(Default)]
struct Board {
    closed: Vec<(u8, Switch)>,
    deadline: Option<u32>,
    leds: u8,
    dry: u8,
    now: u64,
    nvs: BTreeMap<(String, String), Vec<u8>>,
    pattern: Option<LedPattern>,
    reboots: usize,
}

impl NodeSwitches for Board {
    fn assert(&mut self, node: u8, sw: Switch) -> HalResult<()> {
        self.closed.push((node, sw));
        Ok(())
    }
    fn release(&mut self, node: u8, sw: Switch) -> HalResult<()> {
        self.closed.retain(|c| *c != (node, sw));
        Ok(())
    }
    fn release_all(&mut self) -> HalResult<()> {
        self.closed.clear();
        Ok(())
    }
    fn arm_deadline(&mut self, ms: u32) -> HalResult<()> {
        self.deadline = Some(ms);
        Ok(())
    }
    fn disarm_deadline(&mut self) -> HalResult<()> {
        self.deadline = None;
        Ok(())
    }
}

impl NodeSense for Board {
    fn read_leds(&mut self) -> HalResult<u8> {
        Ok(self.leds)
    }
}

impl DryInputs for Board {
    fn read_dry(&mut self) -> HalResult<u8> {
        Ok(self.dry)
    }
}

impl Probes for Board {
    fn scan(&mut self) -> HalResult<Vec<RomId>> {
        Ok(vec![0x28_0000_0000_0001])
    }
    fn read(&mut self, rom: RomId) -> HalResult<i16> {
        if rom == 0x28_0000_0000_0001 {
            Ok(2_500)
        } else {
            Err(HalError::NotPresent)
        }
    }
}

impl BoardTemp for Board {
    fn read_centi_c(&mut self) -> HalResult<i16> {
        Ok(3_000)
    }
}

impl BusVoltage for Board {
    fn read_mv(&mut self) -> HalResult<u32> {
        Ok(19_000)
    }
}

impl StatusLed for Board {
    fn set(&mut self, pattern: LedPattern) -> HalResult<()> {
        self.pattern = Some(pattern);
        Ok(())
    }
}

impl Clock for Board {
    fn now_ms(&self) -> u64 {
        self.now
    }
}

impl Persist for Board {
    fn get(&mut self, namespace: &str, key: &str) -> HalResult<Option<Vec<u8>>> {
        Ok(self.nvs.get(&(namespace.into(), key.into())).cloned())
    }
    fn set(&mut self, namespace: &str, key: &str, value: &[u8]) -> HalResult<()> {
        self.nvs
            .insert((namespace.into(), key.into()), value.to_vec());
        Ok(())
    }
    fn remove(&mut self, namespace: &str, key: &str) -> HalResult<()> {
        self.nvs.remove(&(namespace.into(), key.into()));
        Ok(())
    }
    fn erase_namespace(&mut self, namespace: &str) -> HalResult<()> {
        self.nvs.retain(|(ns, _), _| ns != namespace);
        Ok(())
    }
}

impl Reboot for Board {
    fn request_reboot(&mut self) -> HalResult<()> {
        self.reboots += 1;
        Ok(())
    }
    fn boot_reason(&self) -> BootReason {
        BootReason::PowerOn
    }
}

#[test]
fn a_host_implementation_of_every_trait_works() {
    let mut b = Board {
        leds: 0b1010_1010,
        dry: 0b0011,
        now: 1_234,
        ..Board::default()
    };

    b.assert(1, Switch::Pwr).unwrap();
    b.arm_deadline(750).unwrap();
    assert_eq!(b.deadline, Some(750));
    b.release(1, Switch::Pwr).unwrap();
    b.release_all().unwrap();
    assert!(b.closed.is_empty());

    assert_eq!(b.read_leds().unwrap(), 0b1010_1010);
    assert_eq!(b.read_dry().unwrap(), 0b0011);
    assert_eq!(b.scan().unwrap().len(), 1);
    assert_eq!(b.read(0x28_0000_0000_0001).unwrap(), 2_500);
    assert_eq!(b.read(7), Err(HalError::NotPresent));
    assert_eq!(b.read_centi_c().unwrap(), 3_000);
    assert_eq!(b.read_mv().unwrap(), 19_000);
    assert_eq!(b.now_ms(), 1_234);

    StatusLed::set(&mut b, LedPattern::Heartbeat).unwrap();
    assert_eq!(b.pattern, Some(LedPattern::Heartbeat));

    b.set_blob("cfg", "net", b"{}").unwrap();
    assert_eq!(b.get("cfg", "net").unwrap().unwrap(), b"{}".to_vec());
    b.remove("cfg", "net").unwrap();
    assert_eq!(b.get("cfg", "net").unwrap(), None);
    b.set_blob("secrets", "all", b"x").unwrap();
    b.erase_namespace("secrets").unwrap();
    assert_eq!(b.get("secrets", "all").unwrap(), None);

    b.request_reboot().unwrap();
    assert_eq!(b.reboots, 1);
    assert_eq!(b.boot_reason(), BootReason::PowerOn);
}

/// Small shim so the test body reads clearly next to `StatusLed::set`.
trait SetBlob {
    fn set_blob(&mut self, ns: &str, key: &str, value: &[u8]) -> HalResult<()>;
}

impl SetBlob for Board {
    fn set_blob(&mut self, ns: &str, key: &str, value: &[u8]) -> HalResult<()> {
        Persist::set(self, ns, key, value)
    }
}
