"""Hand route plan for U1's gate drive (pcbkit handroute), U1 rot 0 at (114.45, 116.1).
Top pair (Q3/Q4, phase 2): TG2 / SW2 / BOOST2 on B.Cu over the top of U1, BG2 on B.Cu
under L2 to Q4's gate. Bottom pair (Q1/Q2, phase 1): TG1 and BG1 on In1 lanes under
U1, SW1 on an F.Cu lane along the bottom edge to L1 pad 1, BOOST1 under BG1's via to D1.
Run: pcbkit handroute <board> tools/route_drive.py [--save]
"""
W = 0.25          # drive tracks
V = (0.5, 0.2)    # signal vias
REGION = (106.0, 100.0, 137.0, 123.0)

OPS = [
    # top pair, phase 2
    ('trk', '/TG2', 'B.Cu', [(114.7, 111.938), (114.7, 111.1), (114.3, 110.7), (114.3, 108.8), (114.7, 108.4),
                             (121.2, 108.4), (123.7, 105.9)], W),
    ('trk', '/SW2', 'B.Cu', [(115.2, 111.938), (115.2, 110.2)], W),
    ('trk', '/SW2', 'B.Cu', [(114.97, 110.2), (114.97, 109.4), (115.37, 109.0), (125.8, 109.0)], W),
    ('trk', '/BOOST2', 'B.Cu', [(115.7, 111.938), (115.7, 110.2)], W),
    ('trk', '/BOOST2', 'B.Cu', [(115.93, 110.2), (116.13, 110.0), (120.55, 110.0), (120.9, 110.35), (120.9, 110.65)], W),
    # BG2 stays on B.Cu, no via: a via at Q4's gate would sit 0.2 mm from L1's SW1 pad (F.Cu covers
    # the whole gate area) and an SW1-BG2 short takes out Q4 and U1. Under L2 between D2's pads, In2
    # GND above it, up the right of Q4 to the gate
    ('trk', '/BG2', 'B.Cu', [(117.2, 111.938), (117.2, 111.45), (128.8, 111.45), (131.2, 109.05), (131.2, 105.905)], W),
    ('trk', '/DRVCC', 'B.Cu', [(118.612, 113.85), (120.88, 113.85), (120.88, 112.75)], W),   # pin 26 - C11.1 - D2.2
    # bottom pair, phase 1
    ('trk', '/TG1', 'B.Cu', [(114.7, 120.263), (114.7, 121.1), (114.3, 121.5), (113.85, 121.95)], 0.2),
    ('via', '/TG1', (113.85, 121.95), *V),
    ('trk', '/TG1', 'In1.Cu', [(113.85, 121.95), (113.85, 120.85), (116.7, 118.0), (134.6, 118.0), (135.1, 117.5),
                               (135.1, 116.4)], W),
    ('via', '/TG1', (135.1, 116.4), *V),
    ('trk', '/TG1', 'B.Cu', [(135.1, 116.4), (134.5, 117.0), (134.2, 117.0)], W),
    ('trk', '/BG1', 'B.Cu', [(117.2, 120.263), (117.2, 121.75)], W),
    ('via', '/BG1', (117.2, 121.75), *V),
    # (via left of the Q2 GND vias, below the In1 band y 118.4-119.2 the SENSE1 lines use)
    ('trk', '/BG1', 'In1.Cu', [(117.2, 121.75), (123.6, 121.75), (123.6, 119.6)], W),
    ('via', '/BG1', (123.6, 119.6), *V),
    ('trk', '/BG1', 'B.Cu', [(123.6, 119.6), (123.6, 117.4), (124.0, 117.0), (126.7, 117.0)], W),
    ('trk', '/BOOST1', 'B.Cu', [(115.7, 120.263), (115.7, 121.86)], W),
    ('trk', '/BOOST1', 'B.Cu', [(115.93, 121.86), (115.93, 122.4), (118.95, 122.4), (119.2, 122.15)], W),
    ('trk', '/SW1', 'B.Cu', [(115.2, 120.263), (115.2, 121.86)], W),
    ('trk', '/SW1', 'B.Cu', [(114.97, 121.86), (114.97, 122.33)], W),
    ('via', '/SW1', (114.97, 122.33), *V),
    ('trk', '/SW1', 'F.Cu', [(114.97, 122.33), (128.95, 122.33), (130.0, 121.28), (130.0, 117.9)], W),
    # U1 exposed pad: thermal vias to In2, only right of L2 pad 2 (F.Cu SNS2P above the rest)
] + [('via', 'GND', (115.75, y), 0.6, 0.3) for y in (114.9, 116.1, 117.3)]
