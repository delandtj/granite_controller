"""Build the 8-port plane assembly (FreeCAD) from the KiCad STEP exports.

Run headless:  freecadcmd mech/tools/assemble.py
Input:  mech/step/{pp-48-core,pp-48-unit,pm-19v-250w-b,granite-controller}.step
        (kicad-cli pcb export step --subst-models --no-dnp, KiCad page
        coordinates, STEP Y = -KiCad y, board bottom at z = 0).
Output: mech/assembly/plane-8port.FCStd

Layout (docs/adr/0005-chained-plane-boards.md, decisions 1 and 7):
- core + 4 units in one row, left to right, top edges aligned, GAP apart
  (ADR: ~2-5 mm, the chain cable takes the gap);
- 2 modules per unit on MOD1 / MOD2 (the PM_Module_Outline origin is the
  module centre, = J1 on the module board), rotated as the footprint,
  module B face STANDOFF above the unit F face (docs/pm-interface.md);
- the controller is cable-connected and has no fixed place: it is put
  CTRL_GAP above the row, left-aligned with the core, as a placeholder.

Each instance is its own import of the board STEP inside an App::Part
whose placement moves it (App::Links to one hidden import showed nothing).
"""
import os

import FreeCAD as App
import Import

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
STEP = os.path.join(ROOT, "mech", "step")
OUT = os.path.join(ROOT, "mech", "assembly", "plane-8port.FCStd")

GAP = 3.0          # mm between boards in the row
STANDOFF = 7.0     # module B face to unit F face
PCB_T = 1.6        # unit board thickness
CTRL_GAP = 20.0    # controller placeholder: this far above the row

# board edge boxes in KiCad coordinates (left, top, right, bottom), Edge.Cuts
# centre lines (pcbnew GetBoardEdgesBoundingBox minus the 0.05 half width)
EDGES = {
    "pp-48-core": (52.0, 100.0, 108.0, 172.0),
    "pp-48-unit": (91.0, 100.0, 169.5, 172.0),
    "pm-19v-250w-b": (100.0, 100.0, 158.4, 122.9),
    "granite-controller": (25.0, 28.6, 130.0, 78.6),
}
MOD_CENTRE = (129.2, 111.45)                       # module board J1
MODS = {"MOD1": (112.5, 130.7, -90.0), "MOD2": (137.5, 130.7, -90.0)}
N_UNITS = 4

V, R, P = App.Vector, App.Rotation, App.Placement


def step_xy(x, y):
    return V(x, -y, 0)


def place_board(name, left, top, z=0.0):
    """Placement moving board `name` so its edge box starts at (left, top) in
    STEP coordinates (top = the row's top edge, Y up)."""
    l, t, _, _ = EDGES[name]
    return P(V(left - l, top - (-t), z), R())


def main():
    doc = App.newDocument("plane_8port")
    asm = doc.addObject("App::Part", "Plane_8port")

    def link(name, label, placement):
        """One full import of board `name` per instance (no App::Link: links
        to a hidden source showed nothing in the GUI), moved by `placement`."""
        before = set(doc.Objects)
        Import.insert(os.path.join(STEP, name + ".step"), doc.Name)
        new = [o for o in doc.Objects if o not in before]
        # headless import leaves every feature hidden (no GUI to set it)
        for o in new:
            o.Visibility = True
        part = doc.addObject("App::Part", label)
        part.Label = label
        for o in new:
            if not o.InList:
                part.addObject(o)
        part.Placement = placement
        asm.addObject(part)
        return part

    x = 0.0
    link("pp-48-core", "Core", place_board("pp-48-core", x, 0.0))
    l, _, r, _ = EDGES["pp-48-core"]
    x += (r - l) + GAP
    ul, ut, ur, _ = EDGES["pp-48-unit"]
    mc = step_xy(*MOD_CENTRE)
    port = 0
    for u in range(1, N_UNITS + 1):
        pu = place_board("pp-48-unit", x, 0.0)
        link("pp-48-unit", "Unit%d" % u, pu)
        for ref, (mx, my, rot) in MODS.items():
            port += 1
            # module centre -> origin, footprint rotation, -> MOD position on
            # the unit, then the unit's own move, lifted onto the standoffs
            pm = (pu
                  * P(step_xy(mx, my) + V(0, 0, PCB_T + STANDOFF), R())
                  * P(V(), R(V(0, 0, 1), rot))
                  * P(-mc, R()))
            link("pm-19v-250w-b", "Port%d_U%d_%s" % (port, u, ref), pm)
        x += (ur - ul) + GAP

    cl, ct, cr, cb = EDGES["granite-controller"]
    link("granite-controller", "Controller",
         place_board("granite-controller", 0.0, CTRL_GAP + (cb - ct)))

    doc.recompute()
    os.makedirs(os.path.dirname(OUT), exist_ok=True)
    doc.saveAs(OUT)
    print("saved", OUT, "row length %.1f mm, %d ports" % (x - GAP, port))


main()
