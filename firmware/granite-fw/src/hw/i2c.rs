//! Thin wrapper over the ESP-IDF v5 I2C master driver (`driver/i2c_master.h`).
//!
//! Why not `esp_idf_hal::i2c::I2cDriver`: that one is built on the legacy
//! `driver/i2c.h` API, whose constructor-time conflict check aborts the
//! firmware at startup if the new driver is linked in as well
//! (ESP-IDF v5.5.5, `components/driver/i2c/i2c.c`,
//! `check_i2c_driver_conflict`). The external bus has to use the new
//! driver because the legacy one cannot address the LP I2C port at all, so
//! both buses here go through the new driver and the legacy one is never
//! linked.
//!
//! One [`I2cBus`] per port, one [`I2cDev`] per address. The driver holds a
//! per-bus mutex internally, so devices on the same bus may be used from
//! different threads; this module adds no locking of its own.

use std::ffi::c_int;
use std::sync::Arc;

use esp_idf_sys::{
    EspError, ESP_ERR_INVALID_STATE, ESP_ERR_NOT_FOUND, ESP_ERR_TIMEOUT, esp,
    i2c_addr_bit_len_t_I2C_ADDR_BIT_LEN_7, i2c_clock_source_t, i2c_del_master_bus,
    i2c_device_config_t, i2c_master_bus_add_device, i2c_master_bus_handle_t,
    i2c_master_bus_rm_device, i2c_master_dev_handle_t, i2c_master_probe, i2c_master_transmit,
    i2c_master_transmit_receive, i2c_new_master_bus, i2c_master_bus_config_t,
};
use granite_core::hal::HalError;

/// Transfer timeout for every transaction on an on-board bus. The longest
/// legitimate transfer here is 3 bytes, so anything slower than this is a
/// stuck bus, not a slow device.
pub const XFER_TIMEOUT_MS: c_int = 50;

/// Timeout for a presence probe. An address either acknowledges within one
/// byte time (90 us at 100 kHz) or it does not, so this only has to cover
/// a busy bus. It matters for [`I2cBus::scan`], which probes 112
/// addresses: at the transfer timeout a scan of an empty bus with no
/// pull-ups takes 5.6 s, at this one 1.1 s.
pub const PROBE_TIMEOUT_MS: c_int = 10;

/// Classify an ESP-IDF error for the core's coarse HAL error type.
pub fn hal_err(err: EspError) -> HalError {
    match err.code() {
        ESP_ERR_NOT_FOUND => HalError::NotPresent,
        ESP_ERR_TIMEOUT => HalError::Timeout,
        _ => HalError::Bus,
    }
}

struct BusInner {
    handle: i2c_master_bus_handle_t,
}

// The driver guards the bus with its own mutex and the handle is only ever
// passed back to ESP-IDF.
unsafe impl Send for BusInner {}
unsafe impl Sync for BusInner {}

impl Drop for BusInner {
    fn drop(&mut self) {
        if let Err(err) = esp!(unsafe { i2c_del_master_bus(self.handle) }) {
            log::error!("i2c: deleting bus failed: {err}");
        }
    }
}

/// An I2C master bus on one port.
#[derive(Clone)]
pub struct I2cBus {
    inner: Arc<BusInner>,
    port: i32,
}

impl I2cBus {
    /// Open a bus on `port` with `sda`/`scl` GPIO numbers.
    ///
    /// `clk_source` is an `i2c_clock_source_t` for an HP port and an
    /// `lp_i2c_clock_source_t` for the LP port; both are plain enums of the
    /// same width in the bindings, which is why this takes the raw value.
    /// `internal_pullup` is a safety net only: both buses on this board
    /// have external pull-ups.
    pub fn new(
        port: i32,
        sda: i32,
        scl: i32,
        clk_source: i2c_clock_source_t,
        internal_pullup: bool,
    ) -> Result<Self, EspError> {
        let mut cfg = i2c_master_bus_config_t {
            i2c_port: port,
            sda_io_num: sda,
            scl_io_num: scl,
            glitch_ignore_cnt: 7,
            trans_queue_depth: 0,
            ..Default::default()
        };
        cfg.__bindgen_anon_1.clk_source = clk_source;
        cfg.flags.set_enable_internal_pullup(u32::from(internal_pullup));

        let mut handle: i2c_master_bus_handle_t = std::ptr::null_mut();
        esp!(unsafe { i2c_new_master_bus(&cfg, &mut handle) })?;
        Ok(I2cBus {
            inner: Arc::new(BusInner { handle }),
            port,
        })
    }

    /// Port number this bus runs on.
    pub fn port(&self) -> i32 {
        self.port
    }

    /// Attach a 7-bit device at `addr`, clocked at `scl_hz`.
    pub fn device(&self, addr: u8, scl_hz: u32) -> Result<I2cDev, EspError> {
        let cfg = i2c_device_config_t {
            dev_addr_length: i2c_addr_bit_len_t_I2C_ADDR_BIT_LEN_7,
            device_address: u16::from(addr),
            scl_speed_hz: scl_hz,
            scl_wait_us: 0,
            flags: Default::default(),
        };
        let mut handle: i2c_master_dev_handle_t = std::ptr::null_mut();
        esp!(unsafe { i2c_master_bus_add_device(self.inner.handle, &cfg, &mut handle) })?;
        Ok(I2cDev {
            handle,
            addr,
            _bus: self.inner.clone(),
        })
    }

    /// True when `addr` acknowledges an address-only write.
    pub fn present(&self, addr: u8) -> bool {
        matches!(
            unsafe { i2c_master_probe(self.inner.handle, u16::from(addr), PROBE_TIMEOUT_MS) },
            0
        )
    }

    /// Every 7-bit address in the standard 0x08..=0x77 range that answers.
    pub fn scan(&self) -> Vec<u8> {
        (0x08u8..=0x77).filter(|a| self.present(*a)).collect()
    }
}

/// One addressed device on a bus.
pub struct I2cDev {
    handle: i2c_master_dev_handle_t,
    addr: u8,
    _bus: Arc<BusInner>,
}

// Same reasoning as `BusInner`: the driver serialises access per bus.
unsafe impl Send for I2cDev {}

impl I2cDev {
    /// 7-bit address of this device.
    pub fn addr(&self) -> u8 {
        self.addr
    }

    /// Write `bytes`.
    pub fn write(&self, bytes: &[u8]) -> Result<(), EspError> {
        esp!(unsafe {
            i2c_master_transmit(self.handle, bytes.as_ptr(), bytes.len(), XFER_TIMEOUT_MS)
        })
    }

    /// Write `bytes`, then read `buf` in the same transaction.
    pub fn write_read(&self, bytes: &[u8], buf: &mut [u8]) -> Result<(), EspError> {
        esp!(unsafe {
            i2c_master_transmit_receive(
                self.handle,
                bytes.as_ptr(),
                bytes.len(),
                buf.as_mut_ptr(),
                buf.len(),
                XFER_TIMEOUT_MS,
            )
        })
    }
}

impl Drop for I2cDev {
    fn drop(&mut self) {
        // ESP_ERR_INVALID_STATE here means the bus is already gone, which
        // only happens while the whole Hw struct is being dropped.
        match esp!(unsafe { i2c_master_bus_rm_device(self.handle) }) {
            Ok(()) => {}
            Err(err) if err.code() == ESP_ERR_INVALID_STATE => {}
            Err(err) => log::error!("i2c: removing device 0x{:02x} failed: {err}", self.addr),
        }
    }
}
