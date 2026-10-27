"""Hand route plan for the input end around U1's left side and the
signals that cross under U1 (pcbkit handroute), after tools/route_drive.py.
U1 (B) rot 0 at (114.45, 116.1); R10 (F) upright left of L2 pad 2.
Run: pcbkit handroute <board> tools/route_input.py [--save]

Layers: B for U1's escapes, F under R10 for the Kelvin taps, In1 for the
two nets that cross under U1 (RUN above, NDRV below, in the order of
their pins) and for RUN's run to J2 under the In1 VIN strip.
"""
W = 0.2           # signal tracks
V = (0.5, 0.2)    # signal vias
# U1's compensation nets, as the board names them
NETS = {'/VFB1': 'Net-(U1-VFB1)', '/ITH1': 'Net-(U1-ITH1)', '/ITH1RC': 'Net-(C13-Pad1)',
        '/SS1': 'Net-(U1-TRACK{slash}SS1)', '/ITH2': 'Net-(U1-ITH2)', '/SS2': 'Net-(U1-TRACK{slash}SS2)'}
REGION = (100.0, 100.0, 146.0, 123.0)

OPS = [
    # SENSE2- (pin 12): Kelvin from the inner edge of R10 pad 2, under the body
    ('trk', '/VOUT', 'F.Cu', [(109.0, 106.4), (109.0, 110.3)], W),
    ('via', '/VOUT', (109.0, 110.3), *V),
    ('trk', '/VOUT', 'B.Cu', [(109.0, 110.3), (109.0, 110.9), (110.5, 112.4), (110.5, 113.35)], W),
    # SENSE2+ (pin 13): Kelvin from the inner edge of R10 pad 1
    ('trk', '/SNS2P', 'F.Cu', [(110.0, 111.5), (110.0, 111.1), (110.4, 110.7), (111.2, 110.7)], W),
    ('via', '/SNS2P', (111.2, 110.7), *V),
    ('trk', '/SNS2P', 'B.Cu', [(111.2, 110.7), (111.7, 111.2), (111.7, 111.5)], W),
    # EXTVCC (pin 11) from R10 pad 2
    ('trk', '/VOUT', 'F.Cu', [(108.0, 106.4), (108.0, 110.3)], 0.3),
    ('via', '/VOUT', (108.0, 110.3), *V),
    ('trk', '/VOUT', 'B.Cu', [(108.0, 110.3), (108.0, 112.5), (109.35, 113.85), (109.8, 113.85)], 0.3),
    # INTVCC under the body: pin 14 (VFB2) -> pins 8 and 3; C10 at pin 3
    ('trk', '/INTVCC', 'B.Cu', [(112.2, 111.938), (112.2, 113.6), (111.45, 114.35), (111.45, 117.85),
                                (110.9, 117.85)], W),
    ('trk', '/INTVCC', 'B.Cu', [(111.45, 115.35), (110.9, 115.35)], W),
    ('trk', '/INTVCC', 'B.Cu', [(109.8, 117.85), (108.78, 117.85)], W),
    # SGND pins 4-7 tied on their pads, one via; C10's ground to it
    ('trk', 'GND', 'B.Cu', [(109.8, 115.85), (109.8, 117.35)], W),
    ('trk', 'GND', 'B.Cu', [(109.8, 116.6), (108.5, 116.6)], W),
    ('trk', 'GND', 'B.Cu', [(107.82, 117.85), (107.82, 117.28), (108.5, 116.6)], W),
    ('via', 'GND', (108.5, 116.6), *V),
    # ITH2 / TRACK/SS2 caps over pins 15/16, their ground to a via under R10
    ('trk', '/ITH2', 'B.Cu', [(112.55, 110.13), (112.55, 110.55), (112.7, 110.7), (112.7, 111.3)], W),
    ('trk', '/SS2', 'B.Cu', [(113.6, 110.13), (113.6, 110.6), (113.2, 111.0), (113.2, 111.3)], W),
    ('trk', 'GND', 'B.Cu', [(113.6, 109.17), (110.6, 109.17)], W),
    ('via', 'GND', (110.6, 109.17), *V),
    # NDRV (pin 10, tied to DRVCC): In1 under U1 to the DRVCC track at pin 26
    ('trk', '/DRVCC', 'B.Cu', [(109.8, 114.35), (108.5, 114.35)], W),
    ('via', '/DRVCC', (108.5, 114.35), *V),
    ('trk', '/DRVCC', 'In1.Cu', [(108.5, 114.35), (108.9, 113.95), (119.8, 113.95), (119.9, 113.85)], W),
    ('via', '/DRVCC', (119.9, 113.85), *V),
    # OVLO (pin 9) to the R4/R5 divider between the screws
    ('trk', '/OVLO', 'B.Cu', [(109.8, 114.85), (108.95, 114.85), (108.7, 115.1), (106.9, 115.1), (106.5, 114.7),
                              (106.5, 112.51), (106.19, 112.2)], W),
    ('trk', '/OVLO', 'B.Cu', [(104.975, 111.9), (106.19, 111.9)], W),
    ('trk', 'GND', 'B.Cu', [(107.21, 111.9), (107.21, 110.9)], W),
    ('via', 'GND', (107.21, 110.9), *V),
    # RUN divider: R3 pad 1 into the VIN pour, R3 - R4, via to In1
    ('trk', '/VIN', 'B.Cu', [(101.4, 108.938), (101.4, 108.0)], 0.3),
    ('trk', '/RUN', 'B.Cu', [(101.4, 111.862), (101.4, 111.9), (103.325, 111.9), (103.325, 112.9)], W),
    ('via', '/RUN', (103.325, 112.9), *V),
    # RUN: In1 under U1 (above NDRV) to a via under U1's body, B to pins 29/32
    ('trk', '/RUN', 'In1.Cu', [(103.325, 112.9), (103.575, 113.15), (117.1, 113.15)], W),
    ('via', '/RUN', (117.1, 113.15), *V),
    ('trk', '/RUN', 'B.Cu', [(117.1, 113.15), (117.1, 116.85), (118.0, 116.85)], W),
    ('trk', '/RUN', 'B.Cu', [(117.1, 115.35), (118.0, 115.35)], W),
    # RUN to J2.4: F under L2's body, In1 under the VIN strip, B below J2's pads
    # (round BG2's via; the In1 via between C4 and Q3's drain, below the VOUT strip)
    ('trk', '/RUN', 'F.Cu', [(117.1, 113.15), (117.1, 111.85), (116.6, 111.35), (116.6, 110.15), (117.0, 109.75),
                             (117.0, 105.0)], W),
    ('via', '/RUN', (117.0, 105.0), *V),
    ('trk', '/RUN', 'In1.Cu', [(117.0, 105.0), (117.0, 104.85), (117.3, 104.55), (137.2, 104.55), (137.5, 104.85),
                               (137.5, 106.4)], W),
    ('via', '/RUN', (137.5, 106.4), *V),
    ('trk', '/RUN', 'B.Cu', [(137.5, 106.4), (144.0, 106.4), (144.44, 105.96), (144.44, 104.87)], W),
    # SENSE1-/+ (pins 1/48): out under the body to vias at its edge
    ('trk', '/VOUT', 'B.Cu', [(110.9, 118.85), (111.25, 118.85), (111.6, 118.5)], W),
    ('via', '/VOUT', (111.6, 118.5), *V),
    ('trk', '/SNS1P', 'B.Cu', [(111.7, 119.6), (111.7, 119.4), (112.05, 119.05), (112.45, 119.05)], W),
    ('via', '/SNS1P', (112.45, 119.05), *V),
    # VIN for R1 (VIN_IC filter): In1 from the VIN strip, left of RUN's via and
    # BG2, under L2 past BG2's via, to a via between C11 and R1
    ('trk', '/VIN', 'In1.Cu', [(116.4, 104.0), (116.4, 111.6), (121.9, 111.6)], W),
    ('via', '/VIN', (121.9, 111.6), *V),
    ('trk', '/VIN', 'B.Cu', [(121.9, 111.6), (121.9, 112.9), (122.05, 113.05), (122.05, 115.25), (122.925, 115.25)], W),
    # SENSE1- to R9 pad 2: F under L2 to a via below U1's pin 36, In1 band
    # between TG1 and the Q2 GND vias to a via right of C6 (B under R9 is
    # C5-C8), back on F under R9's body to the inner edge of pad 2
    ('trk', '/VOUT', 'F.Cu', [(111.6, 118.5), (118.6, 118.5), (118.6, 119.6)], W),
    ('via', '/VOUT', (118.6, 119.6), *V),
    ('trk', '/VOUT', 'In1.Cu', [(118.6, 119.6), (118.6, 119.0), (119.1, 118.5), (145.2, 118.5), (146.85, 116.85),
                                (150.8, 116.85)], W),
    ('via', '/VOUT', (150.8, 116.85), *V),
    ('trk', '/VOUT', 'F.Cu', [(150.8, 116.85), (147.0, 116.85), (147.0, 116.1)], W),
    # SENSE1+ to R9 pad 1: F beside SENSE1-, down past BG1's via to the slot
    # between the Q2 GND vias and the SW1 lane, In1 along the bottom edge to a
    # via right of C8, F under R9's body to the inner edge of pad 1
    ('trk', '/SNS1P', 'F.Cu', [(112.45, 119.05), (112.6, 118.9), (118.0, 118.9), (118.0, 121.5), (118.4, 121.9),
                               (126.2, 121.9), (126.5, 121.6)], W),
    ('via', '/SNS1P', (126.5, 121.6), *V),
    ('trk', '/SNS1P', 'In1.Cu', [(126.5, 121.6), (149.4, 121.6), (150.8, 120.2)], W),
    ('via', '/SNS1P', (150.8, 120.2), *V),
    ('trk', '/SNS1P', 'F.Cu', [(150.8, 120.2), (147.0, 120.2), (147.0, 121.1)], W),
    # pins 47/46/45 (VFB1, ITH1, SS1) down and left on B, staggered vias
    ('trk', '/VFB1', 'B.Cu', [(112.2, 120.9), (112.2, 121.3), (111.2, 121.3), (110.9, 121.0)], W),
    ('via', '/VFB1', (110.9, 121.0), *V),
    ('trk', '/ITH1', 'B.Cu', [(112.7, 120.9), (112.7, 121.7), (110.8, 121.7), (110.15, 121.05)], W),
    ('via', '/ITH1', (110.15, 121.05), *V),
    ('trk', '/SS1', 'B.Cu', [(113.2, 120.9), (113.2, 122.1), (108.8, 122.1), (108.4, 121.7), (108.4, 121.45)], W),
    ('via', '/SS1', (108.4, 121.45), *V),
    # compensation network on F
    ('trk', '/VOUT', 'F.Cu', [(107.0, 106.4), (106.6, 106.8), (106.6, 110.6), (106.4, 110.8), (106.4, 113.6), (107.2, 114.4),
                              (107.2, 117.9), (107.74, 118.44), (109.95, 118.44)], W),
    ('trk', '/VFB1', 'F.Cu', [(108.85, 119.46), (109.95, 119.46), (109.95, 120.05), (110.9, 121.0)], W),
    ('trk', '/VFB1', 'F.Cu', [(110.9, 121.0), (111.58, 121.0), (112.09, 121.51), (112.09, 122.2)], W),
    ('trk', '/ITH1', 'B.Cu', [(110.15, 121.05), (109.65, 120.55), (108.68, 120.55)], W),
    ('trk', '/ITH1', 'F.Cu', [(110.15, 121.05), (109.9, 120.8), (107.81, 120.8)], W),
    ('trk', '/ITH1RC', 'F.Cu', [(106.79, 120.8), (106.28, 121.31), (106.28, 122.2)], W),
    ('trk', '/SS1', 'F.Cu', [(108.4, 121.45), (108.4, 122.2)], W),
]

OPS = [(op[0], NETS.get(op[1], op[1])) + tuple(op[2:]) if op[0] in ('trk', 'via') else op for op in OPS]

# krt-route rips and reroutes these two nets despite keep_groups;
# tools/route_post.py puts them back after krt-route, clears the router's
# ground taps in their corner and adds the corner's own ground (POST_GND)
POST_NETS = (NETS['/ITH1'], NETS['/ITH1RC'])
POST_GND = [
    ('trk', 'GND', 'F.Cu', [(107.44, 122.2), (107.44, 121.74), (107.3, 121.6)], W),    # C15
    ('via', 'GND', (107.3, 121.6), *V),
]
