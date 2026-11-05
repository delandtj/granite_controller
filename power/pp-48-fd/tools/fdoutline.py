#!/usr/bin/env python3
"""Board outline as pcbkit draws it (1 mm corner arcs) for a new size, holes left alone.
Usage: fdoutline.py <in.kicad_pcb> <out.kicad_pcb> x0 y0 x1 y1"""
import pcbnew,sys
b=pcbnew.LoadBoard(sys.argv[1]); x0,y0,x1,y1=map(float,sys.argv[3:7])
for d in list(b.GetDrawings()):
    if d.GetLayer()==pcbnew.Edge_Cuts: b.Delete(d)
def seg(a,c):
    s=pcbnew.PCB_SHAPE(b); s.SetShape(pcbnew.SHAPE_T_SEGMENT); s.SetLayer(pcbnew.Edge_Cuts); s.SetWidth(pcbnew.FromMM(0.05))
    s.SetStart(pcbnew.VECTOR2I_MM(*a)); s.SetEnd(pcbnew.VECTOR2I_MM(*c)); b.Add(s)
def arc(st,mid,en):
    s=pcbnew.PCB_SHAPE(b); s.SetShape(pcbnew.SHAPE_T_ARC); s.SetLayer(pcbnew.Edge_Cuts); s.SetWidth(pcbnew.FromMM(0.05))
    s.SetArcGeometry(pcbnew.VECTOR2I_MM(*st),pcbnew.VECTOR2I_MM(*mid),pcbnew.VECTOR2I_MM(*en)); b.Add(s)
r=1.0; k=r*(1-0.70710678)
seg((x0+r,y0),(x1-r,y0)); seg((x1,y0+r),(x1,y1-r)); seg((x1-r,y1),(x0+r,y1)); seg((x0,y1-r),(x0,y0+r))
arc((x0,y0+r),(x0+k,y0+k),(x0+r,y0)); arc((x1-r,y0),(x1-k,y0+k),(x1,y0+r)); arc((x1,y1-r),(x1-k,y1-k),(x1-r,y1)); arc((x0+r,y1),(x0+k,y1-k),(x0,y1-r))
b.Save(sys.argv[2])
