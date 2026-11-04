import pcbnew,sys
b=pcbnew.LoadBoard(sys.argv[2])
for z in list(b.Zones()):
    if z.GetIsRuleArea() and z.GetZoneName().startswith('krt-link'): b.Delete(z)
if sys.argv[1]=='add':
    B=pcbnew.B_Cu; ALL=(pcbnew.F_Cu,pcbnew.In1_Cu,pcbnew.In2_Cu,pcbnew.B_Cu)
    R=[((94.3,118.9,164.2,123.2),(B,)),((94.3,118.9,101.0,136.2),(B,)),((157.4,118.9,164.2,136.2),(B,)),
       ((93.6,128.6,99.0,136.5),ALL),((159.5,128.6,164.9,136.5),ALL),((92.4,108.6,100.3,124.8),ALL)]
    for i,((x0,y0,x1,y1),ls) in enumerate(R):
        z=pcbnew.ZONE(b); z.SetIsRuleArea(True); z.SetZoneName('krt-link %d'%i)
        s=pcbnew.LSET()
        for l in ls: s.AddLayer(l)
        z.SetLayerSet(s); z.SetDoNotAllowTracks(True); z.SetDoNotAllowVias(True); z.SetDoNotAllowPads(False); z.SetDoNotAllowZoneFills(False); z.SetDoNotAllowFootprints(False)
        o=z.Outline(); o.NewOutline()
        for x,y in ((x0,y0),(x1,y0),(x1,y1),(x0,y1)): o.Append(pcbnew.FromMM(x),pcbnew.FromMM(y))
        b.Add(z)
b.Save(sys.argv[3])
