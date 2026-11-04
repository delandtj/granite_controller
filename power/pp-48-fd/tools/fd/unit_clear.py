import sys, pcbnew
b = pcbnew.LoadBoard(sys.argv[1]); nets = set(sys.argv[3:])
t = b.Tracks(); gone = [t[i] for i in range(len(t)) if t[i].GetNetname() in nets and t[i].GetParentGroup() is None]
for it in gone: b.Delete(it)
b.Save(sys.argv[2]); print('cleared', len(gone))
