# pcbkit prune eats the pad stubs whose far end is free (U103.8, U203.8):
# put the +3V3 links back, ending on the existing tracks.
REGION = (140, 160, 204, 168)
OPS = [
 ('trk', '+3V3', 'F.Cu', [(142.4, 165.25), (143.3, 165.25), (143.3, 164.3)], 0.2),
 ('trk', '+3V3', 'F.Cu', [(200.4, 165.25), (201.45, 165.25), (201.4, 165.2)], 0.2),
]
