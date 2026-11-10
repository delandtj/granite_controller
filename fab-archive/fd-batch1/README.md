# FD batch 1 fab sets

JLC fab files for FD batch 1 (controller-fd, pp-48-core-fd, pp-48-unit-fd),
for a re-order of exactly the same boards. The design is tag `fd-batch1`
(c65c4b6, 2026-11-09): production design plus the LCSC fixes made during
the order (unit-fd R19 / R20 / R69 / R70 C25099, core-fd D3-D5 C12624).

Generated 2026-11-10 from a checkout of `fd-batch1` with
`pcbkit jlcfab <board>.kicad_pcb <dir>` in each project. The working
`fab/` dirs are gitignored and core-fd / unit-fd had already been
overwritten by next-revision work, so these are regenerated, not the
uploaded files. Check: controller-fd's set from 2026-11-05 still on disk
matched this one exactly (Gerbers, drill, BOM, CPL, rotations), apart
from timestamp lines. JLC's order page has the files as uploaded and
stays the authority.

Per board: `<name>-gerbers.zip` (upload), `<name>-bom.csv`,
`<name>-cpl.csv`, `<name>-rotations.txt` (applied CPL corrections),
`gerbers/` (the zip's contents).
