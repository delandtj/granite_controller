# Setup page assets

Three files, no framework, no build step, no external resource: the board
is on an isolated network and these are served from the firmware image
(ADR 0001 component 8).

- `index.html` - every page of the setup UI as one document: Status,
  Network, MQTT, Nodes, Rules, Security, Firmware, Maintenance, plus the
  first-setup and login panels.
- `app.js` - the whole client: `/id` on load decides between first setup,
  login and the application; everything else is `fetch` against
  `/api/v1/...`.
- `style.css` - layout and colours.

## dist/ is committed on purpose

`./build.sh` gzips the three files into `dist/*.gz` with `gzip -9 -n`
(no timestamp, so the output is reproducible) and
`granite-fw/src/http.rs` embeds those with `include_bytes!`, serving them
with `Content-Encoding: gzip`.

The gzipped files are committed rather than generated during the cargo
build: `include_bytes!` has to find them when the ESP-IDF build runs, and
the firmware build must not depend on `gzip` being installed or on a
build-script ordering. **Re-run `./build.sh` after editing any asset**,
and commit `dist/` with the change. `./build.sh --check` fails when
`dist/` is stale, which is what CI should call.

The script also enforces the 100 KB uncompressed budget from ADR 0001
("the HTTP assets (gzip'd, under 100 KB)"). Current size: 45 KB raw,
12.6 KB gzipped.

## Running it against the simulator

```
cargo run -p granite-sim -- serve --port 8443
```

`granite-sim` serves these same files, so the page can be developed
without a board. See `granite-sim/README.md`.
