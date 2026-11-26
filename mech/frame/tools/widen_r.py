import os
FRAME = os.path.dirname(os.path.dirname(os.path.abspath(__file__))) + "/"  # mech/frame/
import FreeCAD as App, Part
d = App.openDocument(FRAME + "Frame.FCStd")
sh = d.getObject("Params")
sh.set("A49", "'widen_r"); sh.set("B49", "18"); sh.setAlias("B49", "widen_r")
sh.set("C49", "'Extra frame width on the right (front panel header side): moves the right rail off the board edge, so the headers there take a plug")
sh.set("B37", "=board_w + widen_l + widen_r")
sh.set("C37", "'Overall frame width; the board sits at x = widen_l .. frame_w - widen_r")
sh.set("B46", "=band")
sh.set("C46", "'Width of the top-right gusset (was narrower while the right rail lay under the board; the rail is clear of the board now)")
d.recompute()
n = 0
for nm in ("StandoffSketch", "PilotSketch"):
    o = d.getObject(nm)
    for p, e in list(o.ExpressionEngine):
        if e.strip() == "Params.frame_w - Params.hx":
            o.setExpression(p, "Params.frame_w - Params.widen_r - Params.hx"); n += 1
print("W expressions changed", n)
d.recompute()
print("W bad", [o.Name for o in d.Objects if "Invalid" in o.State or "Touched" in o.State])
fr = d.getObject("FrameBody").Shape
print("W frame", fr.BoundBox, "valid", fr.isValid(), "solids", len(fr.Solids), "vol", round(fr.Volume))
print("W frame_w", sh.get("frame_w"), "pitch", sh.get("pitch"), "board", d.getObject("BoardRef").Shape.BoundBox)
for nm in ("StandoffSketch", "PilotSketch", "GussetTRSketch", "GussetBRSketch"):
    b = d.getObject(nm).Shape.BoundBox; print("W", nm, "x %.1f..%.1f z %.1f..%.1f" % (b.XMin, b.XMax, b.ZMin, b.ZMax))
pitch = float(sh.get("pitch"))
ms = d.getObject("MoboLink").Shape; ps = d.getObject("PlugsLink").Shape
nxt = fr.copy(); nxt.translate(App.Vector(0, pitch, 0))
prv = fr.copy(); prv.translate(App.Vector(0, -pitch, 0))
for src, shp in (("mobo", ms), ("plug", ps)):
    for i, s in enumerate(shp.Solids):
        b = s.BoundBox
        if not (b.XMin < 36 or b.XMax > 178 or b.ZMin < 33 or b.ZMax > 177): continue
        for fn, f in (("own", fr), ("next", nxt), ("prev", prv)):
            if not b.intersect(f.BoundBox): continue
            v = f.common(s).Volume
            if v > 0.05: print("W hit", src, i, fn, round(v, 1), "x %.1f..%.1f y %.1f..%.1f z %.1f..%.1f" % (b.XMin, b.XMax, b.YMin, b.YMax, b.ZMin, b.ZMax))
# plug envelopes over the right-edge headers, 20 mm tall, both sides
for nme, x0, x1, z0, z1, side in (("hdr z50-60 cooler side", 192.7, 197.3, 49.2, 59.8, -1), ("hdr z28-38 cooler side", 193.1, 197.7, 28.5, 37.5, -1), ("hdr z121-132 front side", 192.6, 197.0, 120.9, 132.2, 1), ("hdr z64-77 front side", 192.4, 197.5, 64.0, 76.7, 1)):
    env = Part.makeBox(x1 - x0, 20, z1 - z0, App.Vector(x0, 23 - 20 if side < 0 else 24.6, z0))
    print("W plug", nme, "own", round(fr.common(env).Volume, 1), "next", round(nxt.common(env).Volume, 1), "prev", round(prv.common(env).Volume, 1))
d.save(); print("W SAVED")
