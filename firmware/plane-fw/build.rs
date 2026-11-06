//! Pick the linker script and the pin map for the link feature, and
//! check the firmware's pin table against the generated map.
//!
//! The link feature decides the MCU (ADR 0005 decision 9), so it decides
//! both files:
//!
//! | Feature | Pin map | memory.x |
//! |---|---|---|
//! | `link-i2c` | `plane-core-pins.toml` (rev D) | `memory-g070cb.x` |
//! | `link-mcp` | `plane-core-fd-pins.toml` | `memory-g070cb.x` |
//! | `link-fdcan` | `plane-core-fd-pins.toml` | `memory-g0b1cc.x` |
//!
//! The chosen script is copied to `OUT_DIR/memory.x` and `OUT_DIR` is
//! put on the linker's search path, which is how `INCLUDE memory.x` in
//! cortex-m-rt's `link.x` finds it. There is deliberately no `memory.x`
//! in the crate root: ld resolves an `INCLUDE` from the current
//! directory first, so one there would win over whatever was copied.
//!
//! The pin check: every row of the `core_pins!` table in the build's
//! pin module must name a net of the TOML, on the same port, with the
//! same mode string, and every net of the TOML that is not a fixed
//! debug / crystal / reset pin must appear in the table. A target build
//! cannot assert anything about the board (there is no test harness on
//! it), so this is the pin map's test.
//!
//! Hand-written parsers on purpose: the TOML is generated and trivially
//! regular (`[[pin]]` blocks of `key = value`), and a build dependency
//! would be a dependency more than this check is worth.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

/// Nets the MCU does not drive: debug, crystal and reset.
const FIXED: &[&str] = &["NRST", "SWDIO", "SWCLK", "OSC_IN", "OSC_OUT"];

/// The three link features, with the pin module, the generated pin map
/// and the linker script each one builds against.
const LINKS: &[(&str, &str, &str, &str)] = &[
    (
        "CARGO_FEATURE_LINK_I2C",
        "src/pins.rs",
        "../plane-core-pins.toml",
        "memory-g070cb.x",
    ),
    (
        "CARGO_FEATURE_LINK_MCP",
        "src/pins_fd.rs",
        "../plane-core-fd-pins.toml",
        "memory-g070cb.x",
    ),
    (
        "CARGO_FEATURE_LINK_FDCAN",
        "src/pins_fd.rs",
        "../plane-core-fd-pins.toml",
        "memory-g0b1cc.x",
    ),
];

/// One `[[pin]]` block of the generated map.
#[derive(Default)]
struct TomlPin {
    port: String,
    mode: String,
}

fn main() {
    for (_, pins_rs, pin_map, script) in LINKS {
        println!("cargo:rerun-if-changed={pins_rs}");
        println!("cargo:rerun-if-changed={pin_map}");
        println!("cargo:rerun-if-changed={script}");
    }

    let chosen: Vec<_> = LINKS
        .iter()
        .filter(|(feature, ..)| std::env::var_os(feature).is_some())
        .collect();
    let [(_, pins_rs, pin_map, script)] = chosen[..] else {
        // src/main.rs carries the `compile_error!` that says this in
        // full; a build script error says it without waiting for
        // embassy-stm32, whose own "no chip selected" error would
        // otherwise be the first thing a reader sees.
        println!(
            "cargo::error=plane-fw needs exactly one link feature \
             (link-i2c, link-mcp or link-fdcan); {} are set",
            chosen.len()
        );
        return;
    };

    // The linker script of the MCU this link runs on.
    let out = PathBuf::from(std::env::var_os("OUT_DIR").expect("cargo sets OUT_DIR"));
    std::fs::copy(script, out.join("memory.x"))
        .unwrap_or_else(|e| panic!("plane-fw build script cannot copy {script}: {e}"));
    println!("cargo:rustc-link-search={}", out.display());

    let pin_map = Path::new(pin_map);
    let pins_rs = Path::new(pins_rs);
    let generated = read(pin_map);
    let source = read(pins_rs);

    let mut problems = String::new();
    let toml = parse_pin_map(&generated);
    let code = parse_core_pins(&source);

    let map = pin_map.display();
    let table = pins_rs.display();

    if code.is_empty() {
        let _ = writeln!(
            problems,
            "{table}: no core_pins! rows found, so nothing was checked"
        );
    }

    for (net, port, mode) in &code {
        match toml.get(net) {
            None => {
                let _ = writeln!(
                    problems,
                    "net {net} is not in {map} (the firmware \
                     drives a net the board does not have)"
                );
            }
            Some(pin) => {
                if &pin.port != port {
                    let _ = writeln!(
                        problems,
                        "net {net}: {table} says {port}, {map} says {}",
                        pin.port
                    );
                }
                if &pin.mode != mode {
                    let _ = writeln!(
                        problems,
                        "net {net} ({port}): {table} says mode {mode:?}, \
                         {map} says {:?}",
                        pin.mode
                    );
                }
            }
        }
    }

    for (net, pin) in &toml {
        if FIXED.contains(&net.as_str()) {
            continue;
        }
        if !code.iter().any(|(n, _, _)| n == net) {
            let _ = writeln!(
                problems,
                "net {net} ({}, {}) is in {map} and not in \
                 the core_pins! table",
                pin.port, pin.mode
            );
        }
    }

    if !problems.is_empty() {
        panic!(
            "\nplane-fw pin map does not match firmware/{}:\n{problems}\n\
             The TOML is generated from power/pp-48*/tools/sch/blocks/mcu.py and wins; \
             fix {table} (and whatever uses the pin).\n",
            pin_map.file_name().unwrap_or_default().display()
        );
    }
}

/// Read a file or fail the build with the path in the message.
fn read(path: &Path) -> String {
    std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("plane-fw build script cannot read {}: {e}", path.display()))
}

/// Every `[[pin]]` block of the generated map, by net.
fn parse_pin_map(text: &str) -> BTreeMap<String, TomlPin> {
    let mut pins = BTreeMap::new();
    let mut current: Option<(String, TomlPin)> = None;

    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') {
            if let Some((net, pin)) = current.take() {
                pins.insert(net, pin);
            }
            if line == "[[pin]]" {
                current = Some((String::new(), TomlPin::default()));
            }
            continue;
        }
        let Some((net, pin)) = current.as_mut() else {
            continue;
        };
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = unquote(value.trim());
        match key.trim() {
            "net" => *net = value.to_string(),
            "port" => pin.port = value.to_string(),
            "mode" => pin.mode = value.to_string(),
            _ => {}
        }
    }
    if let Some((net, pin)) = current.take() {
        pins.insert(net, pin);
    }
    pins.remove("");
    pins
}

/// `"value"` without its quotes.
fn unquote(value: &str) -> &str {
    value.trim().trim_matches('"')
}

/// The `NET : PORT = "mode";` rows of the `core_pins!` table.
fn parse_core_pins(source: &str) -> Vec<(String, String, String)> {
    let mut rows = Vec::new();
    let Some(start) = source.find("core_pins! {") else {
        return rows;
    };
    // The table ends at the first line that is a lone closing brace,
    // which is the macro invocation's own.
    let body = &source[start..];
    for line in body.lines().skip(1) {
        let line = line.trim();
        if line == "}" {
            break;
        }
        if !line.ends_with(';') || line.starts_with("//") {
            continue;
        }
        let row = line.trim_end_matches(';');
        let Some((net, rest)) = row.split_once(':') else {
            continue;
        };
        let Some((port, mode)) = rest.split_once('=') else {
            continue;
        };
        rows.push((
            net.trim().to_string(),
            port.trim().to_string(),
            unquote(mode).to_string(),
        ));
    }
    rows
}
