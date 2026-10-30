# Schematic fix (handoff 2026-10-24 finding 1): Device:LED pin 1 = K is now
# GND and pin 2 = A the resistor node (/Bucks/LED12_A, /Bucks/LED33_A,
# /MCU/LEDRUN_A, /Slot n/ONL_A, /Slot n/NL_A). The board had the resistor
# track on pad 1 and GND on pad 2, so each LED turns 180 degrees in place:
# pad 2 lands on the old pad 1 spot under the resistor track, pad 1 on the
# old pad 2 spot under the GND stubs / pour. Run BEFORE `pcbkit relink` with
# the new netlist: until relink the pads still carry the old nets and DRC
# shows the swapped pads as shorts.
REGION = (100.0, 100.0, 250.0, 195.0)
OPS = [
    ('rot', 'D2', 180),
    ('rot', 'D3', 180),
    ('rot', 'D4', 180),
    ('rot', 'D102', 180),
    ('rot', 'D103', 180),
    ('rot', 'D202', 180),
    ('rot', 'D203', 180),
]
