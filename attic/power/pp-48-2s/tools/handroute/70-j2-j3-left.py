# J2 and J3 1.7 mm towards the board centre so the pin legends fit between
# the connector body and the board edge (INT is 2.1 mm wide, connlabels keeps
# 0.3 mm to the edge), J3 0.6 mm up so its courtyard clears C26 and J2 0.5 mm
# up (more and its courtyard hits J5 and H2).
# The pads land on the F.Cu corridor between the isolators and the old pad
# column, so:
#   CTRL_INT   right side on F.Cu at x 245.3 (horizontal stubs into the pads);
#              the B.Cu feeder to J3.5 goes round the SDA1/SCL1 vias on the left
#   CTRL_SCL   right side on PWR (In2) at x 245.3, south leg on F.Cu at 245.3
#   CTRL_SDA   right side on B.Cu at x 245.3 (was F.Cu left of the pads),
#              back to F.Cu below J3 through a via at (243.7, 135.8)
#   EXP_IRQ    right side moves out to x 246.6
#   LED_RUN    corridor run moves from x 243.5 to 242.95
#   CTRL_GND   the via that fed C26 sat where the J3 pads land; new via at
#              (241.95, 132.0) between the SDA1 and SCL1 inner tracks
#   CTRL_3V3   J3.1 fed along y 132.4 instead of the diagonal the LED_RUN
#              via would cut
#   GND        the U4.5 -> U5.5 stitch that wandered to x 243.3 is dropped
#              (both pads keep their own via to the B.Cu pour)
# New pin rows: J2 y 118.0 116.0 114.0 112.0 110.0 (pins 1..5),
#               J3 y 132.4 130.4 128.4 126.4 124.4, pads x 244.1.
REGION = (238.5, 107.0, 250.0, 142.0)
OPS = [
    ('move', 'J2', (-1.7, -0.5)),
    ('move', 'J3', (-1.7, -0.6)),
    # B.Cu feeders from the isolators into the new pad centres
    ('delseg', 'CTRL_3V3', 'B.Cu', (239.6, 118.8, 245.5, 118.8)),
    ('delseg', 'CTRL_3V3', 'B.Cu', (238.4, 117.6, 239.6, 118.8)),
    ('trk', 'CTRL_3V3', 'B.Cu', [(238.4, 117.6), (239.0, 118.2), (244.1, 118.2)], 0.2),
    ('delseg', 'CTRL_GND', 'B.Cu', (241.5, 116.8, 245.5, 116.8)),
    ('trk', 'CTRL_GND', 'B.Cu', [(241.5, 116.8), (242.3, 116.0), (244.1, 116.0)], 0.2),
    ('delseg', '/Link/CTRL_SCL', 'B.Cu', (245.8, 114.5, 242.6, 114.5)),
    ('trk', '/Link/CTRL_SCL', 'B.Cu', [(242.6, 114.5), (243.1, 114.0), (244.1, 114.0)], 0.2),
    ('delseg', '/Link/CTRL_SDA', 'B.Cu', (245.8, 112.5, 241.3, 112.5)),
    ('delseg', '/Link/CTRL_SDA', 'B.Cu', (241.3, 112.5, 239.8, 114.0)),
    ('trk', '/Link/CTRL_SDA', 'B.Cu', [(239.8, 114.0), (241.8, 112.0), (245.3, 112.0)], 0.2),
    ('delseg', '/Link/CTRL_INT', 'B.Cu', (245.8, 125.0, 244.4, 125.0)),
    ('delseg', '/Link/CTRL_INT', 'B.Cu', (244.4, 125.0, 241.2, 128.2)),
    ('trk', '/Link/CTRL_INT', 'B.Cu', [(241.15, 128.2), (241.15, 124.4), (244.1, 124.4)], 0.2),
    # CTRL_INT J2.5 - J3.5, F.Cu right side
    ('delseg', '/Link/CTRL_INT', 'F.Cu', (247.0, 112.1, 245.8, 110.9)),
    ('delseg', '/Link/CTRL_INT', 'F.Cu', (247.0, 123.8, 247.0, 112.1)),
    ('delseg', '/Link/CTRL_INT', 'F.Cu', (245.8, 125.0, 247.0, 123.8)),
    ('trk', '/Link/CTRL_INT', 'F.Cu', [(244.1, 110.0), (245.3, 110.0), (245.3, 124.4), (244.1, 124.4)], 0.2),
    # CTRL_SCL J2.3 - J3.3 on PWR, south leg on F.Cu
    ('delseg', '/Link/CTRL_SCL', 'PWR', (247.0, 115.7, 245.8, 114.5)),
    ('delseg', '/Link/CTRL_SCL', 'PWR', (247.0, 127.4, 247.0, 115.7)),
    ('delseg', '/Link/CTRL_SCL', 'PWR', (245.8, 128.6, 247.0, 127.4)),
    ('trk', '/Link/CTRL_SCL', 'PWR', [(244.1, 114.0), (245.3, 114.0), (245.3, 128.4), (244.1, 128.4)], 0.2),
    ('delseg', '/Link/CTRL_SCL', 'F.Cu', (247.0, 130.6, 245.8, 129.4)),
    ('delseg', '/Link/CTRL_SCL', 'F.Cu', (247.0, 134.4, 247.0, 130.6)),
    ('delseg', '/Link/CTRL_SCL', 'F.Cu', (242.7, 138.7, 247.0, 134.4)),
    ('trk', '/Link/CTRL_SCL', 'F.Cu', [(244.1, 128.4), (245.3, 128.4), (245.3, 136.1), (242.7, 138.7)], 0.2),
    # CTRL_SDA: B.Cu right side, back to F.Cu below J3
    ('delseg', '/Link/CTRL_SDA', 'F.Cu', (244.6, 114.1, 245.8, 112.9)),
    ('delseg', '/Link/CTRL_SDA', 'F.Cu', (244.6, 125.6, 244.6, 114.1)),
    ('delvia', '/Link/CTRL_SDA', (244.6, 125.6)),
    ('delseg', '/Link/CTRL_SDA', 'PWR', (245.8, 126.8, 244.6, 125.6)),
    ('delseg', '/Link/CTRL_SDA', 'F.Cu', (244.6, 128.6, 245.8, 127.4)),
    ('delseg', '/Link/CTRL_SDA', 'F.Cu', (244.6, 134.3, 244.6, 128.6)),
    ('delseg', '/Link/CTRL_SDA', 'F.Cu', (242.7, 136.2, 244.6, 134.3)),
    ('delseg', '/Link/CTRL_SDA', 'F.Cu', (242.3, 136.2, 242.7, 136.2)),
    ('trk', '/Link/CTRL_SDA', 'B.Cu', [(245.3, 112.0), (245.3, 135.4), (244.9, 135.8), (243.7, 135.8)], 0.2),
    ('trk', '/Link/CTRL_SDA', 'B.Cu', [(244.1, 126.4), (245.3, 126.4)], 0.2),
    ('via', '/Link/CTRL_SDA', (243.7, 135.8), 0.5, 0.2),
    ('trk', '/Link/CTRL_SDA', 'F.Cu', [(243.7, 135.8), (243.3, 136.2), (242.3, 136.2)], 0.2),
    # EXP_IRQ out to x 246.6
    ('delseg', '/EXP_IRQ', 'F.Cu', (245.1, 108.3, 247.8, 111.0)),
    ('delseg', '/EXP_IRQ', 'F.Cu', (247.8, 111.0, 247.8, 134.7)),
    ('delseg', '/EXP_IRQ', 'F.Cu', (247.8, 134.7, 242.5, 140.0)),
    ('trk', '/EXP_IRQ', 'F.Cu', [(245.1, 108.3), (246.6, 109.8), (246.6, 135.9), (242.5, 140.0)], 0.2),
    # CTRL_GND: J3.2 feeder and the C26 link
    ('delseg', 'CTRL_GND', 'B.Cu', (239.6, 129.8, 244.6, 129.8)),
    ('delseg', 'CTRL_GND', 'B.Cu', (244.6, 129.8, 245.8, 131.0)),
    ('delvia', 'CTRL_GND', (243.0, 129.8)),
    ('delseg', 'CTRL_GND', 'F.Cu', (243.0, 129.8, 242.5, 130.3)),
    ('delseg', 'CTRL_GND', 'F.Cu', (242.5, 130.3, 242.5, 135.5)),
    ('trk', 'CTRL_GND', 'B.Cu', [(239.6, 129.8), (242.9, 129.8), (243.5, 130.4), (244.1, 130.4)], 0.2),
    ('trk', 'CTRL_GND', 'B.Cu', [(241.95, 129.8), (241.95, 132.0)], 0.2),
    ('via', 'CTRL_GND', (241.95, 132.0), 0.5, 0.2),
    ('trk', 'CTRL_GND', 'F.Cu', [(241.95, 132.0), (242.48, 132.53), (242.48, 135.5)], 0.2),
    # CTRL_3V3 J3.1 feeder
    ('delseg', 'CTRL_3V3', 'B.Cu', (241.7, 134.6, 244.2, 134.6)),
    ('delseg', 'CTRL_3V3', 'B.Cu', (244.2, 134.6, 245.8, 133.0)),
    ('trk', 'CTRL_3V3', 'B.Cu', [(241.7, 134.6), (241.7, 133.2), (242.5, 132.4), (244.1, 132.4)], 0.2),
    # LED_RUN corridor run
    ('delseg', '/MCU/LED_RUN', 'F.Cu', (240.3, 126.2, 243.5, 129.4)),
    ('delseg', '/MCU/LED_RUN', 'F.Cu', (243.5, 129.4, 243.5, 133.7)),
    ('delseg', '/MCU/LED_RUN', 'F.Cu', (243.5, 133.7, 243.1, 134.1)),
    ('trk', '/MCU/LED_RUN', 'F.Cu', [(240.3, 126.2), (242.95, 128.85), (242.95, 133.95), (243.1, 134.1)], 0.2),
    # GND stitch U4.5 -> U5.5 through x 243.3
    ('delreg', 'GND', 'F.Cu', (241.0, 110.1, 243.5, 116.3)),
    ('delseg', 'GND', 'F.Cu', (240.91, 110.22, 240.9, 110.2)),
    ('delseg', 'GND', 'PWR', (243.0, 116.1, 241.5, 117.6)),
]
