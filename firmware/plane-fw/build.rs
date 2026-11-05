//! Check the firmware's pin map against the generated one.
//!
//! `firmware/plane-core-pins.toml` comes out of the schematic generator
//! and is the truth for the PP-48 core board. A target build cannot
//! assert anything about it (there is no test harness on the board), so
//! the check runs here: every row of the `core_pins!` table in
//! `src/pins.rs` must name a net of the TOML, on the same port, with
//! the same mode string, and every net of the TOML that is not a fixed
//! debug / crystal / reset pin must appear in the table.
//!
//! Hand-written parsers on purpose: the TOML is generated and trivially
//! regular (`[[pin]]` blocks of `key = value`), and a build dependency
//! would be a dependency more than this check is worth.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;

/// Nets the MCU does not drive: debug, crystal and reset.
const FIXED: &[&str] = &["NRST", "SWDIO", "SWCLK", "OSC_IN", "OSC_OUT"];

/// One `[[pin]]` block of the generated map.
#[derive(Default)]
struct TomlPin {
    port: String,
    mode: String,
}

fn main() {
    let pin_map = Path::new("../plane-core-pins.toml");
    let pins_rs = Path::new("src/pins.rs");
    println!("cargo:rerun-if-changed={}", pin_map.display());
    println!("cargo:rerun-if-changed={}", pins_rs.display());
    println!("cargo:rerun-if-changed=memory.x");

    let generated = read(pin_map);
    let source = read(pins_rs);

    let mut problems = String::new();
    let toml = parse_pin_map(&generated);
    let code = parse_core_pins(&source);

    if code.is_empty() {
        let _ = writeln!(
            problems,
            "src/pins.rs: no core_pins! rows found, so nothing was checked"
        );
    }

    for (net, port, mode) in &code {
        match toml.get(net) {
            None => {
                let _ = writeln!(
                    problems,
                    "net {net} is not in plane-core-pins.toml (the firmware \
                     drives a net the board does not have)"
                );
            }
            Some(pin) => {
                if &pin.port != port {
                    let _ = writeln!(
                        problems,
                        "net {net}: pins.rs says {port}, plane-core-pins.toml says {}",
                        pin.port
                    );
                }
                if &pin.mode != mode {
                    let _ = writeln!(
                        problems,
                        "net {net} ({port}): pins.rs says mode {mode:?}, \
                         plane-core-pins.toml says {:?}",
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
                "net {net} ({}, {}) is in plane-core-pins.toml and not in \
                 the core_pins! table",
                pin.port, pin.mode
            );
        }
    }

    if !problems.is_empty() {
        panic!(
            "\nplane-fw pin map does not match firmware/plane-core-pins.toml:\n{problems}\n\
             The TOML is generated from power/pp-48/tools/sch/blocks/mcu.py and wins; \
             fix src/pins.rs (and whatever uses the pin).\n"
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
