import os
FRAME = os.path.dirname(os.path.dirname(os.path.abspath(__file__))) + "/"  # mech/frame/
import FreeCAD as App
d = App.openDocument(FRAME + "Frame.FCStd")
ms = d.getObject("MoboLink").Shape
hs = d.getObject("HaloStack")
cx, cz = hs.Placement.Base.x, hs.Placement.Base.z
print("K centre", cx, cz, "board face y 23")
rows = []
for i, s in enumerate(ms.Solids):
    b = s.BoundBox
    if b.YMax > 23.05 or b.YLength < 0.3: continue
    if b.XMax < cx - 45 or b.XMin > cx + 45 or b.ZMax < cz - 45 or b.ZMin > cz + 45: continue
    h = 23 - b.YMin
    if h < 1.5: continue
    rows.append((h, i, b.XMin - cx, b.XMax - cx, b.ZMin - cz, b.ZMax - cz))
rows.sort(reverse=True)
for r in rows[:40]:
    inped = (r[2] < 17.5 and r[3] > -17.5 and r[4] < 17.5 and r[5] > -17.5)
    print("K h %.1f  #%d  dx %.1f..%.1f dz %.1f..%.1f %s" % (r[0], r[1], r[2], r[3], r[4], r[5], "UNDER PEDESTAL" if inped else ""))
print("K count >=1.5", len(rows))
# holes near the 75 mm pattern corners: look for cylindrical faces through the board? report parts at corners
for sx in (-37.5, 37.5):
    for sz in (-37.5, 37.5):
        near = [(round(23 - s.BoundBox.YMin, 1)) for s in ms.Solids if s.BoundBox.YMax <= 23.05 and s.BoundBox.XMin < cx + sx + 4 and s.BoundBox.XMax > cx + sx - 4 and s.BoundBox.ZMin < cz + sz + 4 and s.BoundBox.ZMax > cz + sz - 4 and 23 - s.BoundBox.YMin > 0.5]
        print("K corner", sx, sz, "parts within 4 mm, heights", sorted(near, reverse=True)[:6])
