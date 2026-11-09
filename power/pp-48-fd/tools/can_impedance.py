"""CANH / CANL differential impedance on pp-48-unit-fd (B.Cu over In2, JLC04161H-7628), 2026-11-09.

Run: uv run --no-project --with scipy --with numpy python -I can_impedance.py
Cross-section at x 105 / 150 taken from the board (GND pour edges, LINK_GND, AUX_12V).
"""
import os
import sys
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from xsec import impedances

base = dict(h=0.2104, er=4.4, t=0.035, mask_t=0.01, mask_er=3.8, ztop=4.0)


def tr(name, yc, w):
    return (name, yc - w / 2, yc + w / 2)


def show(label, g, **kw):
    r = impedances(g, **kw)
    if "Zdiff" in r:
        print(f"{label:58s} Zdiff {r['Zdiff']:6.1f}  Zodd {r['Zodd']:5.1f}  Zeven {r['Zeven']:5.1f}  "
              f"Zse {r['Zse'][0]:5.1f}/{r['Zse'][1]:5.1f}  eeff_odd {r['eeff_odd']:.2f}  grid {r['grid']}")
    else:
        print(f"{label:58s} Z0 {list(v for k, v in r.items() if k.startswith('Z0'))[0]:6.1f}  eeff {r['eeff']:.2f}")
    return r


# --- sanity: a textbook microstrip, 0.36 mm on 0.2104 / 4.4, no mask (~50 ohm expected)
g = dict(base, mask_t=0.0, span=(-5, 5), rects=[tr("S", 0, 0.36)])
show("check: 0.36 mm microstrip, no mask (expect ~50)", g)

# --- unit-fd at x=105 / 150 (GND pour to 119.0, from 123.16), y in board mm
rects = [("G", 112.0, 119.0), tr("CANL", 119.6, 0.2), tr("CANH", 120.4, 0.2),
         tr("LINK_GND", 121.4, 0.5), tr("AUX_12V", 122.4, 0.5), ("G", 123.16, 128.0)]
# CANH / CANL must be the first two signal names (sorted): rename to keep order
rects = [(n.replace("CANH", "A_CANH").replace("CANL", "A_CANL").replace("LINK_GND", "G")
          .replace("AUX_12V", "G"), a, b) for n, a, b in rects]
g = dict(base, span=(112.0, 128.0), rects=rects)
show("main run (x 105/150): pair + LINK_GND/AUX_12V/pours AC-gnd", g)
show("  same, grid refined", g, fine=0.002, coarse=0.05)
g2 = dict(g, mask_t=0.02)
show("  mask 0.02 mm instead of 0.01", g2)
g3 = dict(g, er=4.1)
show("  er 4.1 (7628 at higher frequency)", g3)

# --- isolated pair (no side copper) for reference
g = dict(base, span=(112.0, 128.0), rects=[tr("A_CANL", 119.6, 0.2), tr("A_CANH", 120.4, 0.2)])
show("isolated pair 0.2 / 0.6 gap", g)

# --- vertical runs at the ends: 2.54 mm apart, effectively uncoupled
g = dict(base, span=(112.0, 128.0), rects=[tr("A_CANL", 117.46, 0.2), tr("A_CANH", 120.0, 0.2)])
show("end runs, 2.54 mm c-c", g)

# --- what geometry gives ~120 ohm differential (pair alone, mask 0.01)
for w, gap in [(0.15, 0.6), (0.12, 0.6), (0.1, 0.6), (0.12, 1.0), (0.1, 1.0), (0.15, 1.2)]:
    g = dict(base, span=(110.0, 130.0), rects=[tr("A_CANL", 120 - (w + gap) / 2, w), tr("A_CANH", 120 + (w + gap) / 2, w)])
    show(f"candidate w {w} gap {gap}", g)
