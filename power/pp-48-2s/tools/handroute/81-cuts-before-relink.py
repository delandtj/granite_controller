# Copper cuts and part moves for the schematic fixes 2 and 3 (handoff
# 2026-10-24), run BEFORE `pcbkit relink` with the new netlist so every
# remaining copper cluster touches pads of one new net only. Net names are
# the OLD ones (the board before relink). Run after 80-leds-rotate.py; the
# new copper follows in 82-ripple-cb.py and 83-lclr.py (new names).
#
# Ripple network (LM5164 type 3): CA (C5 / C12) is now INJ -> VOUT, CB
# (C6 / C13) INJ -> FB. Each pair of 0402 caps sat in a row on y 118 with
# INJ on the west pad of the west cap, FB on its east pad and VOUT on the
# east pad of the east cap. Swapping the two caps' places (4.0 mm) keeps
# those three pads on their nets: CB moves west (INJ | FB), CA moves east
# (INJ | VOUT). What changes is the east cap's west pad: FB before, INJ now.
#   buck 1: the FB1 tail from C6.2 east (F.Cu, via 157.9/116.5, B.Cu, via
#           159.2/117.0, the F.Cu loop into C6.1) goes;
#   buck 2: the straight FB2 track C12.2 -> C13.1 goes.
# Latch clear: the MCU line Sn_LCLR now drives the Q4 gate (old LCLR_D
# copper: Q.1 + R12.1) and the latch node is /Slot n/CLRN.
#   slot 1: the MCU line reached the latch at via (144.4, 166.2) on PWR; the
#           PWR leg (144.4,166.2)-(144.8,166.6) goes, the latch cluster
#           keeps everything west of it;
#   slot 2: the MCU line came down F.Cu from via (216.4, 159.5) and ran
#           diagonally into C205.1 (also overlapping the latch via at
#           (210.1, 165.2)); that diagonal, its F.Cu lead-in and the
#           PWR jog via (216.4, 159.5) go. The MCU line now ends at via
#           (216.4, 158.5).
REGION = (100.0, 100.0, 250.0, 195.0)
OPS = [
    ('move', 'C5', (4.0, 0.0)),
    ('move', 'C6', (-4.0, 0.0)),
    ('move', 'C12', (4.0, 0.0)),
    ('move', 'C13', (-4.0, 0.0)),
    # buck 1: FB1 tail east of the (new) C6.2
    ('delseg', '/Bucks/FB1', 'F.Cu', (155.0, 118.0, 156.4, 118.0)),
    ('delseg', '/Bucks/FB1', 'F.Cu', (156.4, 118.0, 157.9, 116.5)),
    ('delvia', '/Bucks/FB1', (157.9, 116.5)),
    ('delseg', '/Bucks/FB1', 'B.Cu', (157.9, 116.5, 158.4, 117.0)),
    ('delseg', '/Bucks/FB1', 'B.Cu', (158.4, 117.0, 159.2, 117.0)),
    ('delvia', '/Bucks/FB1', (159.2, 117.0)),
    ('delseg', '/Bucks/FB1', 'F.Cu', (159.2, 117.0, 159.6, 117.4)),
    ('delseg', '/Bucks/FB1', 'F.Cu', (159.6, 117.4, 159.6, 118.3)),
    ('delseg', '/Bucks/FB1', 'F.Cu', (159.6, 118.3, 159.2, 118.7)),
    ('delseg', '/Bucks/FB1', 'F.Cu', (159.2, 118.7, 158.7, 118.7)),
    ('delseg', '/Bucks/FB1', 'F.Cu', (158.7, 118.7, 158.0, 118.0)),
    # buck 2: FB2 link between the two caps
    ('delseg', '/Bucks/FB2', 'F.Cu', (198.98, 118.0, 202.02, 118.0)),
    # slot 1: MCU line off the latch via
    ('delseg', '/S1_LCLR', 'PWR', (144.4, 166.2, 144.8, 166.6)),
    # slot 2: MCU line off C205.1
    ('delseg', '/S2_LCLR', 'F.Cu', (214.9, 160.6, 209.5, 166.0)),
    ('delseg', '/S2_LCLR', 'F.Cu', (215.3, 160.6, 214.9, 160.6)),
    ('delseg', '/S2_LCLR', 'F.Cu', (216.4, 159.5, 215.3, 160.6)),
    ('delseg', '/S2_LCLR', 'PWR', (216.4, 159.5, 216.4, 158.5)),
    ('delvia', '/S2_LCLR', (216.4, 159.5)),
]
