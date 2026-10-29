//! The firmware update over the link and its probation ladder
//! (ADR 0002 decision 9, ADR 0003 FW 0x90-0x94).

#![cfg(feature = "sim")]

use granite_proto::{
    BLK_STATUS, Fw, FwBegin, FwData, INDEX_NONE, ImageState, Info, Status, StatusFlags, UpdateState,
    Version,
};
use plane_logic::crc32;
use plane_logic::plane::{LedPattern, PROBATION_MS};
use plane_logic::sim::{FakeUpdateStore, Fakes, Sim};

/// An image of `len` bytes that is not all 0xFF, so a half-written slot
/// cannot pass its CRC by accident.
fn image(len: usize) -> Vec<u8> {
    (0..len).map(|i| (i * 31 + 7) as u8).collect()
}

/// The update state and next offset STATUS reports.
fn progress(sim: &mut Sim) -> (UpdateState, u32) {
    let payload = sim.read_block(BLK_STATUS, INDEX_NONE);
    let p = Status::records(&payload)
        .map(Result::unwrap)
        .find_map(|r| match r {
            Status::UpdateProgress(p) => Some(p),
            _ => None,
        })
        .expect("STATUS carries the update progress");
    (p.state, p.next_offset)
}

fn settled() -> Sim {
    let mut sim = Sim::with_units(1);
    sim.boot();
    sim.advance(500);
    sim
}

#[test]
fn a_whole_image_goes_over_the_link_and_is_marked_for_the_swap() {
    let mut sim = settled();
    let image = image(1_024);
    let crc = crc32::crc32(&image);
    assert_eq!(progress(&mut sim), (UpdateState::Idle, 0));

    sim.fw_write(&[Fw::Begin(FwBegin {
        size: image.len() as u32,
        crc32: crc,
        version: Version::new(0, 2, 0),
    })]);
    assert_eq!(progress(&mut sim), (UpdateState::Receiving, 0));
    assert!(
        sim.plane
            .status_flags()
            .contains(StatusFlags::UPDATE_IN_PROGRESS)
    );
    assert_eq!(sim.plane.led_pattern(), LedPattern::Fast);
    assert!(sim.saw(granite_proto::EventKind::UpdateStateChanged));

    // The controller paces itself on the offset STATUS gives back.
    for chunk in image.chunks(granite_proto::FW_DATA_MAX) {
        let (state, offset) = progress(&mut sim);
        assert_eq!(state, UpdateState::Receiving);
        sim.fw_write(&[Fw::Data(FwData {
            offset,
            data: chunk,
        })]);
    }
    assert_eq!(progress(&mut sim), (UpdateState::Receiving, 1_024));

    sim.fw_write(&[Fw::Commit { crc32: crc }]);
    assert_eq!(progress(&mut sim), (UpdateState::Ready, 1_024));
    assert!(sim.dev.update.swap_marked, "the bootloader will swap");
    assert_eq!(sim.dev.update.image(1_024), &image[..]);

    // A reboot performs it; the intended state goes first.
    assert_eq!(
        sim.ctrl_now(1, &[granite_proto::Command::Reboot]),
        granite_proto::ResultCode::Ok
    );
    assert!(sim.plane.reset_requested());
}

#[test]
fn a_bad_crc_fails_the_commit_and_the_swap_is_not_marked() {
    let mut sim = settled();
    let image = image(512);
    sim.fw_write(&[Fw::Begin(FwBegin {
        size: image.len() as u32,
        crc32: crc32::crc32(&image),
        version: Version::new(0, 2, 0),
    })]);
    // One record per 128 bytes, the most the wire carries.
    for (i, chunk) in image.chunks(granite_proto::FW_DATA_MAX).enumerate() {
        sim.fw_write(&[Fw::Data(FwData {
            offset: (i * granite_proto::FW_DATA_MAX) as u32,
            data: chunk,
        })]);
    }
    // The controller's own CRC disagrees with what arrived.
    sim.fw_write(&[Fw::Commit { crc32: 0xDEAD_BEEF }]);
    assert_eq!(progress(&mut sim).0, UpdateState::Failed);
    assert!(!sim.dev.update.swap_marked);
    assert!(
        !sim.plane
            .status_flags()
            .contains(StatusFlags::UPDATE_IN_PROGRESS)
    );
}

#[test]
fn an_image_larger_than_the_slot_is_refused() {
    let mut sim = settled();
    let capacity = sim.dev.update.dfu.len() as u32;
    sim.fw_write(&[Fw::Begin(FwBegin {
        size: capacity + 1,
        crc32: 0,
        version: Version::new(9, 9, 9),
    })]);
    assert_eq!(progress(&mut sim).0, UpdateState::Failed);
}

#[test]
fn a_repeated_data_record_is_harmless_and_a_gap_is_refused() {
    let mut sim = settled();
    let image = image(256);
    let crc = crc32::crc32(&image);
    sim.fw_write(&[Fw::Begin(FwBegin {
        size: 256,
        crc32: crc,
        version: Version::new(0, 2, 0),
    })]);
    sim.fw_write(&[Fw::Data(FwData {
        offset: 0,
        data: &image[..128],
    })]);
    assert_eq!(progress(&mut sim), (UpdateState::Receiving, 128));

    // The controller did not see the acknowledgement and sends the same
    // record again: taken, offset unchanged.
    sim.fw_write(&[Fw::Data(FwData {
        offset: 0,
        data: &image[..128],
    })]);
    assert_eq!(progress(&mut sim), (UpdateState::Receiving, 128));

    // A record that would leave a hole is refused and the offset stays
    // where the plane wants it.
    sim.fw_write(&[Fw::Data(FwData {
        offset: 200,
        data: &image[200..256],
    })]);
    assert_eq!(progress(&mut sim), (UpdateState::Receiving, 128));

    // Going back to the offset STATUS asked for finishes the transfer.
    sim.fw_write(&[Fw::Data(FwData {
        offset: 128,
        data: &image[128..],
    })]);
    sim.fw_write(&[Fw::Commit { crc32: crc }]);
    assert_eq!(progress(&mut sim).0, UpdateState::Ready);
}

#[test]
fn an_abort_leaves_the_plane_idle() {
    let mut sim = settled();
    sim.fw_write(&[Fw::Begin(FwBegin {
        size: 256,
        crc32: 0,
        version: Version::new(0, 2, 0),
    })]);
    sim.fw_write(&[Fw::Abort]);
    assert_eq!(progress(&mut sim), (UpdateState::Idle, 0));
    assert!(!sim.dev.update.swap_marked);
}

#[test]
fn an_unconfirmed_image_is_confirmed_by_the_controller() {
    // The first boot after a swap: the image runs on probation.
    let mut sim = Sim::new(Fakes {
        update: FakeUpdateStore::on_probation(),
        ..Fakes::with_units(1)
    });
    sim.boot();
    sim.advance(500);
    assert_eq!(sim.plane.image_state(), ImageState::Probation);
    assert!(
        sim.plane
            .status_flags()
            .contains(StatusFlags::UNCONFIRMED_IMAGE)
    );
    assert_eq!(sim.plane.led_pattern(), LedPattern::Double);
    let payload = sim.read_block(granite_proto::BLK_INFO, INDEX_NONE);
    assert!(
        Info::records(&payload)
            .map(Result::unwrap)
            .any(|r| r == Info::ImageState(ImageState::Probation))
    );

    // The controller checked the plane and confirms.
    sim.fw_write(&[Fw::Confirm]);
    assert!(sim.dev.update.booted);
    assert_eq!(sim.plane.image_state(), ImageState::Confirmed);
    assert!(sim.saw(granite_proto::EventKind::ImageConfirmed));
    assert_eq!(sim.plane.led_pattern(), LedPattern::Heartbeat);

    // And the probation clock is gone: no reset is ever asked for.
    sim.advance(PROBATION_MS + 5_000);
    assert!(!sim.plane.reset_requested());
}

#[test]
fn a_probation_nobody_confirms_asks_for_a_reset() {
    let mut sim = Sim::new(Fakes {
        update: FakeUpdateStore::on_probation(),
        ..Fakes::with_units(1)
    });
    sim.boot();
    sim.advance(PROBATION_MS - 1_000);
    assert!(
        !sim.plane.reset_requested(),
        "still inside the 120 s window"
    );
    sim.advance(2_000);
    assert!(
        sim.plane.reset_requested(),
        "no confirm: reset, and the bootloader swaps back"
    );
    assert!(!sim.dev.update.booted, "the image was never marked good");
}

#[test]
fn a_rollback_is_reported_once() {
    let mut sim = Sim::new(Fakes {
        update: FakeUpdateStore {
            rolled_back: true,
            ..FakeUpdateStore::new()
        },
        ..Fakes::with_units(1)
    });
    sim.boot();
    assert_eq!(sim.plane.image_state(), ImageState::RolledBack);
    assert!(sim.saw(granite_proto::EventKind::RolledBack));
}

#[test]
fn a_store_that_cannot_write_fails_the_update_rather_than_the_plane() {
    let mut sim = settled();
    sim.dev.update.broken = true;
    sim.fw_write(&[Fw::Begin(FwBegin {
        size: 256,
        crc32: 0,
        version: Version::new(0, 2, 0),
    })]);
    assert_eq!(progress(&mut sim).0, UpdateState::Failed);
    // The plane keeps running: the units are still polled.
    sim.advance(1_000);
    assert!(sim.plane.unit(0).unwrap().present());
    assert_eq!(sim.plane.slot(0).unwrap().volts, 4_800);
}
