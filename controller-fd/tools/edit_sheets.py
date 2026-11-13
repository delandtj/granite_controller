import re, uuid
def tops(t):
    """(start,end) of top-level blocks (one tab indent)."""
    res=[]
    for m in re.finditer(r'\n\t\((?=[a-z_])',t):
        p=m.start()+1; d=0
        for j in range(p,len(t)):
            if t[j]=='(':d+=1
            elif t[j]==')':
                d-=1
                if d==0: res.append((p,j+1)); break
    return res
def remove(t,pred):
    bl=[(a,b) for a,b in tops(t) if pred(t[a:b])]
    for a,b in reversed(bl):
        # also drop the preceding newline
        t=t[:a-1]+t[b:]
    return t,len(bl)
def is_sym_ref(refs):
    def f(b):
        if not b.startswith('\t(symbol'): return False
        m=re.search(r'\(property "Reference" "([^"]+)"',b)
        return m and m.group(1) in refs
    return f
def has_uuid(us):
    def f(b):
        m=re.search(r'\n\t\t\(uuid "([^"]+)"\)',b)
        return m and m.group(1) in us and not b.startswith('\t(symbol')
    return f

# ---- power sheet
p='schematic/power.kicad_sch'; t=open(p).read()
t,n=remove(t,is_sym_ref({'J11','J12','D11','#PWR1101','#PWR1102'})); print('power syms',n)
t,n=remove(t,has_uuid({'abd49673-1a47-4122-8701-a056294d64f7','6d182ba0-1da4-44d0-89de-4db95c0ac58a',
  '4f1358b9-a526-4bb2-91bf-0d62e89ad7f8','102bba19-4da9-4da8-9187-c444ef9dbee2','e4b01129-501d-44e1-9447-61db585d3067',
  'f6afddef-cff1-4ca5-a366-2a2abdb0ae94'})); print('power wires',n)
t,n=remove(t,lambda b: b.startswith('\t(label "12V_IN_B"') or (b.startswith('\t(label "VIN_12V"') and '(at 72.39 139.7' in b)); print('power labels',n)
# J11 wire now starts at the AUX_12V label
assert '(xy 25.4 114.3) (xy 59.69 114.3)' in t
t=t.replace('(xy 25.4 114.3) (xy 59.69 114.3)','(xy 30.48 114.3) (xy 59.69 114.3)')
# label 12V_IN_A -> global label AUX_12V
a,b=[(a,b) for a,b in tops(t) if t[a:b].startswith('\t(label "12V_IN_A"')][0]
u=re.search(r'\(uuid "([^"]+)"\)',t[a:b]).group(1)
gl=('\t(global_label "AUX_12V"\n\t\t(shape input)\n\t\t(at 30.48 114.3 180)\n\t\t(fields_autoplaced yes)\n'
    '\t\t(effects\n\t\t\t(font\n\t\t\t\t(size 1.27 1.27)\n\t\t\t)\n\t\t\t(justify right)\n\t\t)\n'
    '\t\t(uuid "%s")\n\t\t(property "Intersheetrefs" "${INTERSHEET_REFS}"\n\t\t\t(at 30.48 114.3 0)\n'
    '\t\t\t(effects\n\t\t\t\t(font\n\t\t\t\t\t(size 1.27 1.27)\n\t\t\t\t)\n\t\t\t\t(justify right)\n\t\t\t\t(hide yes)\n\t\t\t)\n\t\t)\n\t)')%u
t=t[:a]+gl+t[b:]
old='12 V input: two plane aux feeds (plane J4: PH 2p, 1 CTRL_12V, 2 CTRL_GND) -> one SS34 each (OR-ing + reverse polarity) -> SMAJ15A -> LMR51430 buck (1.1 MHz) -> 5.2 V. No fuse: the plane iso module limits its own output.'
assert old in t
t=t.replace(old,'12 V input: AUX_12V from the plane link box header J15 pins 5 / 6 (OR-ed from every core, ADR 0006 decision 4) -> SS34 D10 (reverse polarity) -> SMAJ15A -> LMR51430 buck (1.1 MHz) -> 5.44 V (BUCK_5V). No fuse: the cores\' iso modules limit their own output. GND = LINK_GND.')
t=t.replace('(title "Granite controller rev D - ','(title "Granite controller FD - ')
open(p,'w').write(t)

# ---- MCU sheet
p='schematic/mcu.kicad_sch'; t=open(p).read()
for old,new,shape in (('EXP_INT','CAN_STB','output'),('PLANE_SDA','CAN_nCS','output'),('PLANE_SCL','CAN_nINT','input')):
    a,b=[(a,b) for a,b in tops(t) if t[a:b].startswith('\t(global_label "%s"'%old)][0]
    blk=t[a:b].replace('(global_label "%s"'%old,'(global_label "%s"'%new).replace('(shape bidirectional)','(shape %s)'%shape)
    t=t[:a]+blk+t[b:]
for old,new in (('Rev D: GPIO2/3 plane link (HP I2C), GPIO6/7 local I2C, GPIO10/11/4/15 dry inputs 1-4, GPIO0 EXP_INT',
                 'FD: GPIO2 / GPIO3 MCP2518FD nCS / nINT (CAN link, SPI shared with the W5500), GPIO0 TCAN1044V STB, GPIO6/7 local I2C, GPIO10/11/4/15 dry inputs 1-4'),
                ('Plane link pull-ups (2.2k) and EXP_INT 10k on the Plane link sheet.','CAN_nCS / CAN_nINT 10k pull-ups on the CAN link sheet.')):
    assert old in t, old; t=t.replace(old,new)
t=t.replace('(title "Granite controller rev D - ','(title "Granite controller FD - ')
open(p,'w').write(t)

# ---- root
p='granite-controller-fd.kicad_sch'; t=open(p).read()
t=t.replace('(property "Sheetname" "Plane link"','(property "Sheetname" "CAN link"').replace('"schematic/planelink.kicad_sch"','"schematic/canlink.kicad_sch"')
open(p,'w').write(t)
