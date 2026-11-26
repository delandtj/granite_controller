import os
FRAME = os.path.dirname(os.path.dirname(os.path.abspath(__file__))) + "/"  # mech/frame/
import FreeCAD as App, Part
V = App.Vector
d = App.openDocument(FRAME + "Frame.FCStd")
sh = d.getObject("Params"); pitch = float(sh.get("pitch"))
ms = d.getObject("MoboLink").Shape
hs = d.getObject("HaloStack"); cx, cz = hs.Placement.Base.x, hs.Placement.Base.z
fr = d.getObject("FrameBody").Shape
TOP = 1.7 + 26.5           # K129 top above the board's cooler face
# K129 envelope on this board: base 90 x 90, from the chip top to its fin tops
k = Part.makeBox(90, TOP - 1.7, 90, V(cx - 45, 23 - TOP, cz - 45))
print("B k129 y %.1f..%.1f  (rail back is y 0), sticks out %.1f" % (23 - TOP, 21.3, TOP - 23))
print("B clash with own frame", round(fr.common(k).Volume, 1))
prv = fr.copy(); prv.translate(V(0, -pitch, 0))
print("B clash with previous frame body", round(prv.common(k).Volume, 1))
# previous board: its solder side faces us. In its own coords, front parts have y > 24.6
rows = []
for i, s in enumerate(ms.Solids):
    b = s.BoundBox
    if b.YMax <= 24.65: continue
    if b.XMax < cx - 45 or b.XMin > cx + 45 or b.ZMax < cz - 45 or b.ZMin > cz + 45: continue
    h = b.YMax - 24.6
    rows.append((h, i, b.XMin - cx, b.XMax - cx, b.ZMin - cz, b.ZMax - cz))
rows.sort(reverse=True)
gap = pitch - 24.6          # previous board's solder face to our rail back
print("B gap solder face -> next rail back %.1f, K129 takes %.1f, leaves %.1f for parts" % (gap, TOP - 23, gap - (TOP - 23)))
for r in rows[:14]:
    print("B   h %.1f #%d dx %.1f..%.1f dz %.1f..%.1f  clearance to K129 %.1f" % (r[0], r[1], r[2], r[3], r[4], r[5], gap - (TOP - 23) - r[0]))
print("B parts on the solder side under the footprint:", len(rows))
# infra frame behind the first node
i = App.openDocument(FRAME + "InfraFrame.FCStd")
for o in i.Objects:
    if o.TypeId != "Part::Feature": continue
    s = o.Shape.copy(); s.translate(V(0, -36, 0))
    v = s.common(k).Volume
    if v > 0.01:
        c = s.common(k).BoundBox
        print("B infra clash", o.Name, round(v, 1), "x %.1f..%.1f y %.1f..%.1f z %.1f..%.1f" % (c.XMin, c.XMax, c.YMin + 36, c.YMax + 36, c.ZMin, c.ZMax))
