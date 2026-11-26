import os
FRAME = os.path.dirname(os.path.dirname(os.path.abspath(__file__))) + "/"  # mech/frame/
import time
import FreeCAD as App, Part
d = App.openDocument(FRAME + "Frame.FCStd")
sh = d.getObject("Params")
def run(ch):
    sh.set("B35", str(ch)); d.recompute()
    fr = d.getObject("FrameBody").Shape
    mb = d.getObject("MoboLink").Shape
    pl = d.getObject("PlugsLink").Shape
    so = float(sh.get("standoff_h")); pitch = float(sh.get("pitch"))
    by = 12 + so
    print("R cooler_h %s standoff %.1f pitch %.1f board y %.1f mobo y %.1f..%.1f" % (ch, so, pitch, by, mb.BoundBox.YMin, mb.BoundBox.YMax))
    fb = fr.BoundBox
    hits = []
    for src, shp in (("mobo", mb), ("plug", pl)):
        for i, s in enumerate(shp.Solids):
            b = s.BoundBox
            # only parts on the cooler side that reach into the rail / band depth, or near standoffs
            if b.YMin > by - 0.05: continue
            if b.YMin > 12.0 and not (b.XMax > 182 or b.XMin < 16): pass
            try:
                v = fr.common(s).Volume
            except Exception as e:
                v = -1
            if v > 0.01 or v < 0:
                hits.append((src, i, round(v, 2), "x %.1f..%.1f y %.1f..%.1f z %.1f..%.1f" % (b.XMin, b.XMax, b.YMin, b.YMax, b.ZMin, b.ZMax)))
    print("R   hits", len(hits))
    for h in hits[:25]: print("R    ", h)
t = time.time()
for ch in (23, 20, 18, 16, 14):
    run(ch); print("R   t", round(time.time() - t))
