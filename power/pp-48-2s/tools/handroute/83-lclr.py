# Latch clear after 81 and relink: the MCU line Sn_LCLR to the Q4 gate /
# R12 pull-down (the old LCLR_D stub, y 178.6), the latch node CLRN stays
# where it was.
# slot 1: from the cut end of the MCU PWR run at (144.8, 166.6) south-west
#         between the GND vias (143.3, 167.3) and (145.0, 167.2), down
#         x 144.2, diagonal past the OUT via (140.0, 173.4), west along
#         y 177.6 (below the CLRN via at (134.9, 176.1), above the S1_LED
#         PWR diagonal) to a via at (131.6, 177.6), F.Cu down into the
#         Q104.1 / R112.1 track.
# slot 2: the MCU line ends at via (216.4, 158.5); PWR south along x 216.4
#         (east of the GND via column at x 214.95), west along y 177.7
#         (above the CLRN via at (211.3, 178.5); the +3V3 PWR trunk at
#         x 207.4 is the west limit) to a via at (208.4, 177.7), F.Cu down
#         into the Q204.1 / R212.1 track. The latch via (210.1, 165.2),
#         which the old MCU diagonal joined to C205.1, gets its own F.Cu
#         stub into C205.1.
REGION = (126.0, 155.0, 220.0, 184.0)
OPS = [
    ('trk', '/S1_LCLR', 'PWR', [(144.8, 166.6), (144.2, 167.2), (144.2, 173.0), (139.6, 177.6), (131.6, 177.6)], 0.2),
    ('via', '/S1_LCLR', (131.6, 177.6), 0.5, 0.2),
    ('trk', '/S1_LCLR', 'F.Cu', [(131.6, 177.6), (131.6, 178.6)], 0.2),
    ('trk', '/S2_LCLR', 'PWR', [(216.4, 158.5), (216.4, 175.0), (213.7, 177.7), (208.4, 177.7)], 0.2),
    ('via', '/S2_LCLR', (208.4, 177.7), 0.5, 0.2),
    ('trk', '/S2_LCLR', 'F.Cu', [(208.4, 177.7), (208.4, 178.6)], 0.2),
    ('trk', '/Slot 2/CLRN', 'F.Cu', [(210.1, 165.2), (209.5, 165.8)], 0.2),
]
