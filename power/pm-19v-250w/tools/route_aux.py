"""J2 is now the 1.27 mm aux header (2026-10-27, docs/pm-interface.md): its
pads sit on x 139.9 / 143.9, y 102.33 / 103.6 / 104.87, GND in the middle
row. The 1.5 mm gap between the pad columns (x 141.15-142.65) takes two
vias: PGOOD level with the top row (its old via sat inside J2.6), and GND
in the middle row tying J2.5 and J2.6 to the In2 plane (the B pour cannot
get between the rows). The old B jumper that joined the 2.54 mm GND pads
goes; ID gets the 0.25 mm it lacked to its new pad. OV_KILL and RUN already
end inside their new pads.
Run: pcbkit handroute pm-19v-250w.kicad_pcb tools/route_aux.py [--save]
(after krt-route / route_post.py, before gndpour)
"""
REGION = (134.0, 98.0, 150.0, 112.0)
OPS = [
    ('delreg', 'GND', 'B.Cu', (136.5, 100.4, 142.2, 105.0)),
    ('delvia', '/PGOOD', (143.1, 103.7)),
    ('delseg', '/PGOOD', 'B.Cu', (143.1, 103.7, 144.4, 102.4)),
    ('delseg', '/PGOOD', 'SIG', (143.1, 109.6, 143.1, 103.7)),
    ('trk', '/PGOOD', 'SIG', [(143.1, 109.6), (143.1, 103.53), (141.9, 102.33)], 0.2),
    ('via', '/PGOOD', (141.9, 102.33), 0.5, 0.2),
    ('trk', '/PGOOD', 'B.Cu', [(141.9, 102.33), (143.0, 102.33)], 0.2),
    ('via', 'GND', (141.9, 103.6), 0.5, 0.2),
    ('trk', 'GND', 'B.Cu', [(139.9, 103.6), (143.9, 103.6)], 0.3),
    ('trk', '/ID', 'B.Cu', [(138.4, 104.9), (139.2, 104.9)], 0.2),
]
