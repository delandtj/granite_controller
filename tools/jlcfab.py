#!/usr/bin/env python3
"""JLCPCB fabrication and assembly outputs.

Usage: jlcfab.py [board.kicad_pcb] [out_dir]     (run with the KiCad python)

Writes to out_dir (default fab/):
  gerbers/                 4-layer Gerbers (Protel extensions, no X2/netlist,
                           soldermask subtracted from silk) + Excellon drill
                           (PTH and NPTH separate, mm, absolute origin)
  <name>-gerbers.zip       the above, ready to upload
  <name>-bom.csv           Comment, Designator, Footprint, LCSC Part #
  <name>-cpl.csv           Designator, Mid X, Mid Y, Layer, Rotation
  <name>-rotations.txt     parts whose rotation was corrected, to check in
                           JLC's placement preview

DNP footprints and footprints without an LCSC field (bare test pads) are
left out of the BOM and CPL. CPL coordinates are absolute board mm with Y
flipped, same as the Gerbers. Through-hole parts use the centre of their
pads (their footprint origin is usually pin 1); SMD parts use the
footprint origin.
"""
import csv
import os
import re
import shutil
import subprocess
import sys
import zipfile

import pcbnew

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
pcb = sys.argv[1] if len(sys.argv) > 1 else os.path.join(ROOT, 'granite_controller.kicad_pcb')
out = sys.argv[2] if len(sys.argv) > 2 else os.path.join(ROOT, 'fab')
name = os.path.splitext(os.path.basename(pcb))[0]

# JLC's zero orientation differs from KiCad's for some packages. Subset of
# the community table in matthewlai/JLCKicadTools (cpl_rotations_db.csv),
# only the footprint types used on this board; first match wins.
ROTATIONS = [
    (r'^R_Array_Convex_', 90),
    (r'^SOT-23', -90),
    (r'^LQFP-', 270),
    (r'^SOP-(?!18_)', 270),
    (r'^SOIC-', 270),
    (r'^USON-10', 270),
    (r'^CP_EIA-', 180),
]

LAYERS = 'F.Cu,In1.Cu,In2.Cu,B.Cu,F.Paste,B.Paste,F.Silkscreen,B.Silkscreen,F.Mask,B.Mask,Edge.Cuts'

gdir = os.path.join(out, 'gerbers')
shutil.rmtree(gdir, ignore_errors=True)
os.makedirs(gdir)


def run(*args):
    r = subprocess.run(['kicad-cli', *args], capture_output=True, text=True)
    if r.returncode != 0:
        sys.exit('kicad-cli %s failed:\n%s%s' % (args[2], r.stdout, r.stderr))


run('pcb', 'export', 'gerbers', '--layers', LAYERS, '--no-x2', '--no-netlist',
    '--subtract-soldermask', '--check-zones', '-o', gdir + '/', pcb)
run('pcb', 'export', 'drill', '--format', 'excellon', '--excellon-separate-th',
    '--excellon-units', 'mm', '--excellon-zeros-format', 'decimal', '--drill-origin', 'absolute',
    '--generate-map', '--map-format', 'gerberx2', '-o', gdir + '/', pcb)

zpath = os.path.join(out, name + '-gerbers.zip')
with zipfile.ZipFile(zpath, 'w', zipfile.ZIP_DEFLATED) as z:
    for f in sorted(os.listdir(gdir)):
        z.write(os.path.join(gdir, f), f)

board = pcbnew.LoadBoard(pcb)
mm = pcbnew.ToMM
bom = {}
cpl = []
corrected = []
skipped = []
for fp in sorted(board.GetFootprints(), key=lambda f: f.GetReference()):
    ref = fp.GetReference()
    lcsc = fp.GetFieldText('LCSC').strip() if fp.HasField('LCSC') else ''
    if fp.IsDNP() or not lcsc:
        skipped.append('%s (%s)' % (ref, 'DNP' if fp.IsDNP() else 'no LCSC'))
        continue
    fpname = fp.GetFPID().GetLibItemName().wx_str()
    mpn = fp.GetFieldText('MPN').strip() if fp.HasField('MPN') else ''
    e = bom.setdefault(lcsc, {'refs': [], 'values': set(), 'fp': fpname, 'mpn': mpn})
    e['refs'].append(ref)
    e['values'].add(fp.GetValue())

    pads = list(fp.Pads())
    # footprint attribute, not pads: a module's thermal pad has plated vias
    tht = bool(fp.GetAttributes() & pcbnew.FP_THROUGH_HOLE)
    if tht and pads:
        xs = [mm(p.GetPosition().x) for p in pads]
        ys = [mm(p.GetPosition().y) for p in pads]
        x, y = (min(xs) + max(xs)) / 2, (min(ys) + max(ys)) / 2
    else:
        x, y = mm(fp.GetPosition().x), mm(fp.GetPosition().y)
    rot = fp.GetOrientationDegrees()
    for pat, corr in ROTATIONS:
        if re.match(pat, fpname):
            corrected.append('%s %s: %g -> %g' % (ref, fpname, rot % 360, (rot + corr) % 360))
            rot += corr
            break
    side = 'Bottom' if fp.IsFlipped() else 'Top'
    cpl.append([ref, '%.4fmm' % x, '%.4fmm' % -y, side, '%g' % (rot % 360)])

bom_path = os.path.join(out, name + '-bom.csv')
with open(bom_path, 'w', newline='') as f:
    w = csv.writer(f, quoting=csv.QUOTE_ALL)
    w.writerow(['Comment', 'Designator', 'Footprint', 'LCSC Part #'])
    natural = lambda r: (re.sub(r'\d+', '', r), int(re.sub(r'\D', '', r) or 0))
    for lcsc, e in sorted(bom.items(), key=lambda kv: natural(sorted(kv[1]['refs'], key=natural)[0])):
        # one value -> use it; several (e.g. per-channel connector names) -> the MPN
        comment = next(iter(e['values'])) if len(e['values']) == 1 else (e['mpn'] or sorted(e['values'])[0])
        w.writerow([comment, ','.join(sorted(e['refs'], key=natural)), e['fp'], lcsc])

cpl_path = os.path.join(out, name + '-cpl.csv')
with open(cpl_path, 'w', newline='') as f:
    w = csv.writer(f, quoting=csv.QUOTE_ALL)
    w.writerow(['Designator', 'Mid X', 'Mid Y', 'Layer', 'Rotation'])
    w.writerows(cpl)

with open(os.path.join(out, name + '-rotations.txt'), 'w') as f:
    f.write('Rotation corrections applied (KiCad -> JLC). Check these in the\n'
            'JLC placement preview; the table is community-maintained.\n\n')
    f.write('\n'.join(corrected) + '\n')

print('gerbers + drill:', len(os.listdir(gdir)), 'files ->', zpath)
print('BOM:', len(bom), 'lines,', sum(len(e['refs']) for e in bom.values()), 'parts ->', bom_path)
print('CPL:', len(cpl), 'placements,', len(corrected), 'rotation-corrected ->', cpl_path)
print('left out:', ', '.join(skipped))
