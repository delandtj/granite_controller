# Ripple network copper after the C5/C6 and C12/C13 swap (81) and relink.
# Row on y 118, west to east: CB.1 INJ | CB.2 FB | CA.1 INJ | CA.2 VOUT.
# buck 1 (CB = C6 at x 154.5, CA = C5 at x 158.5):
#   INJ1  over the top of C6: C6.1 up to y 117.3, east, down into C5.1
#         (the old +3V3 dog-leg into C5.2 used y 117.3, so it goes);
#   +3V3  from the via at (156.8, 118.6) under C5.1 at y 118.6 into C5.2.
# buck 2 (CB = C13 at x 198.5, CA = C12 at x 202.5):
#   INJ2  from the INJ2 track west of C13 (x 197.4, left of the FB2 via at
#         (198.0, 117.2)) up to y 116.5, east above the FB2 via and below the
#         S1_TEMP via at (199.4, 115.8), down into C12.1.
REGION = (148.0, 113.0, 206.0, 121.0)
OPS = [
    ('delseg', '+3V3', 'F.Cu', (156.8, 118.6, 156.8, 118.2)),
    ('delseg', '+3V3', 'F.Cu', (156.8, 118.2, 157.7, 117.3)),
    ('delseg', '+3V3', 'F.Cu', (157.7, 117.3, 158.3, 117.3)),
    ('delseg', '+3V3', 'F.Cu', (158.3, 117.3, 159.0, 118.0)),
    ('delseg', '+3V3', 'F.Cu', (159.0, 118.0, 158.98, 118.0)),
    ('trk', '+3V3', 'F.Cu', [(156.8, 118.6), (158.68, 118.6), (158.98, 118.3)], 0.2),
    ('trk', '/Bucks/INJ1', 'F.Cu', [(154.02, 118.0), (154.02, 117.3), (158.02, 117.3), (158.02, 118.0)], 0.2),
    ('trk', '/Bucks/INJ2', 'F.Cu', [(197.4, 118.0), (197.4, 116.5), (202.02, 116.5), (202.02, 118.0)], 0.2),
]
