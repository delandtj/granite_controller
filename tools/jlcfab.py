#!/usr/bin/env python3
"""JLCPCB fabrication and assembly outputs.

Usage: jlcfab.py [board.kicad_pcb] [out_dir]     (run with the KiCad python)

Writes to out_dir (default fab/):
  gerbers/                 4-layer Gerbers (Protel extensions, no X2/netlist,
                           soldermask subtracted from silk) + Excellon drill
                           (PTH and NPTH separate, mm, absolute origin; no
                           drill-map Gerbers, JLC would read them as layers)
  <name>-gerbers.zip       the above, ready to upload
  <name>-bom.csv           Comment, Designator, Footprint, LCSC Part #
  <name>-cpl.csv           Designator, Mid X, Mid Y, Layer, Rotation
  <name>-rotations.txt     parts whose rotation or centroid was corrected,
                           plus any part missing from CORRECTIONS

DNP footprints and footprints without an LCSC field (bare test pads) are
left out of the BOM and CPL. CPL coordinates are absolute board mm with Y
flipped, same as the Gerbers. Through-hole parts use the centre of their
pads (their footprint origin is usually pin 1); SMD parts use the
footprint origin. CORRECTIONS then moves that to the EasyEDA origin.
"""
import csv
import math
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

# JLC places each part with its EasyEDA/LCSC library footprint: the CPL
# position is that footprint's origin and rotation 0 is its orientation.
# Entries below were derived by fetching the EasyEDA footprint for each LCSC
# number and fitting its numbered pads onto the KiCad pads at rotation 0
# (pin 1 / cathode / + checked against the EasyEDA symbol pin names and silk
# marks). Matched as (footprint regex, LCSC or None, rotation, dx, dy); first
# match wins. dx, dy (mm, KiCad frame at rotation 0, y down) is where the
# EasyEDA origin sits relative to our reference point (footprint origin for
# SMD, pad centre for THT); it is rotated with the part before use.
# Re-verify when an LCSC number changes: the same KiCad footprint can map to
# different EasyEDA layouts (U2/U8 vs U4 SOT-23).
CORRECTIONS = [
    # SOT-23: EasyEDA layouts differ per part
    (r'^SOT-23-6$', 'C7519', 270, 0, 0),          # U4 USBLC6-2SC6
    (r'^SOT-23-6$', 'C5210749', 180, 0, 0),       # U8 LMR51430, pin 1 lower right
    (r'^SOT-23-5$', 'C141836', 180, 0, 0),        # U2 TLV62569, pin 1 lower right
    (r'^LQFP-48_7x7mm_P0.5mm$', None, 270, 0, 0),
    (r'^SOIC-8_3.9x4.9mm_P1.27mm$', None, 270, 0, 0),
    (r'^SOIC-28W_7.5x17.9mm_P1.27mm$', None, 270, 0, 0),
    (r'^SOP-16_4.55x10.3mm_P1.27mm$', None, 270, 0, 0),
    (r'^SOIC-4_4.55x3.7mm_P2.54mm$', None, 0, 0, 0),   # photoMOS: same layout as KiCad
    (r'^USON-10_2.5x1.0mm_P0.5mm$', None, 270, 0, 0),
    # symmetric 4x0402 array: 90 and 270 are both correct, 0/180 are not
    (r'^R_Array_Convex_4x0402$', None, 90, 0, 0),
    # polarity: EasyEDA pad 1 = + on the tantalum-polymer (silk + mark)
    (r'^CP_EIA-7343-31_Kemet-D$', None, 180, 0, 0),
    # EasyEDA pad 1 = cathode (symbol pin K/C, silk bar) -> no change
    (r'^D_SMA$', None, 0, 0, 0),
    (r'^D_SOD-123F$', None, 0, 0, 0),
    (r'^D_SOD-923$', None, 0, 0, 0),
    (r'^LED_0603_1608Metric$', None, 0, 0, 0),
    (r'^Crystal_SMD_Abracon_ABM8G-4Pin_3.2x2.5mm$', None, 0, 0, 0),
    (r'^L_Sunlord_SWPA3015S$', None, 0, 0, 0),    # unpolarized; EasyEDA is 180
    (r'^L_Changjiang_FXL0630$', None, 0, 0, 0),
    # EasyEDA origin = centre of the pad array, KiCad origin = module centre
    (r'^ESP32-C6-WROOM-1$', None, 0, 0, 2.995),
    # EasyEDA origin 1.308 mm towards the signal-pad row from KiCad's
    (r'^USB_C_Receptacle_GCT_USB4105-xx-A_16P_TopMnt_Horizontal$', None, 0, 0, -1.308),
    (r'^RJ45_Hanrun_HR911105A_Horizontal$', None, 0, 0, 0),
    # JST PH vertical: EasyEDA pin 1 is at the +x end, origin at pad centre
    (r'^JST_PH_B\dB-PH-K_1x\d\d_P2.00mm_Vertical$', None, 180, 0, 0),
]
# Two-terminal unpolarized parts need no entry.
UNPOLARIZED = r'^(R|C|L|Fuse)_\d{4}_\d{4}Metric$'

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
    '-o', gdir + '/', pcb)

zpath = os.path.join(out, name + '-gerbers.zip')
with zipfile.ZipFile(zpath, 'w', zipfile.ZIP_DEFLATED) as z:
    for f in sorted(os.listdir(gdir)):
        z.write(os.path.join(gdir, f), f)

board = pcbnew.LoadBoard(pcb)
mm = pcbnew.ToMM
bom = {}
cpl = []
corrected = []
unverified = []
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
    for pat, part, corr, dx, dy in CORRECTIONS:
        if re.match(pat, fpname) and part in (None, lcsc):
            if dx or dy:
                if fp.IsFlipped():
                    sys.exit('%s: centroid offset on a bottom-side part, not handled' % ref)
                # KiCad orientation is CCW on screen with y down
                a = math.radians(rot)
                x += dx * math.cos(a) + dy * math.sin(a)
                y += -dx * math.sin(a) + dy * math.cos(a)
            if corr or dx or dy:
                corrected.append('%s %s %s: rot %g -> %g, offset (%g, %g)'
                                 % (ref, fpname, lcsc, rot % 360, (rot + corr) % 360, dx, dy))
            rot += corr
            break
    else:
        if not re.match(UNPOLARIZED, fpname):
            unverified.append('%s %s %s' % (ref, fpname, lcsc))
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
    f.write('Rotation/centroid corrections applied (KiCad -> JLC), derived from\n'
            'the EasyEDA footprints. Still check them in the JLC placement preview.\n\n')
    f.write('\n'.join(corrected) + '\n')
    if unverified:
        f.write('\nNOT IN THE TABLE (written uncorrected, verify):\n')
        f.write('\n'.join(unverified) + '\n')

print('gerbers + drill:', len(os.listdir(gdir)), 'files ->', zpath)
print('BOM:', len(bom), 'lines,', sum(len(e['refs']) for e in bom.values()), 'parts ->', bom_path)
print('CPL:', len(cpl), 'placements,', len(corrected), 'rotation-corrected ->', cpl_path)
print('left out:', ', '.join(skipped))
if unverified:
    print('WARNING: no rotation entry for:', ', '.join(unverified))
