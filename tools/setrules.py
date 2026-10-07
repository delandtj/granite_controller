#!/usr/bin/env python3
"""Apply the JLCPCB JLC04161H-7628 4L 1.6 mm stackup (1 oz outer, 0.5 oz inner) to
the .kicad_pcb and rules/netclasses to the .kicad_pro.

Diff pairs (2D field solve, surface microstrip over In1 GND, 0.2104 mm 7628):
ETH 0.16/0.15 = 101 ohm in fluid (Er 2.1 above the mask), 106 ohm in air.
USB 0.25/0.15 = 89 ohm in air (service port, used out of the fluid).
"""
import json, sys
pcb, pro = sys.argv[1:3]

STACKUP = '''\t\t(stackup
\t\t\t(layer "F.SilkS" (type "Top Silk Screen"))
\t\t\t(layer "F.Paste" (type "Top Solder Paste"))
\t\t\t(layer "F.Mask" (type "Top Solder Mask") (thickness 0.01))
\t\t\t(layer "F.Cu" (type "copper") (thickness 0.035))
\t\t\t(layer "dielectric 1" (type "prepreg") (thickness 0.2104) (material "FR4 7628") (epsilon_r 4.4) (loss_tangent 0.02))
\t\t\t(layer "In1.Cu" (type "copper") (thickness 0.0152))
\t\t\t(layer "dielectric 2" (type "core") (thickness 1.065) (material "FR4") (epsilon_r 4.6) (loss_tangent 0.02))
\t\t\t(layer "In2.Cu" (type "copper") (thickness 0.0152))
\t\t\t(layer "dielectric 3" (type "prepreg") (thickness 0.2104) (material "FR4 7628") (epsilon_r 4.4) (loss_tangent 0.02))
\t\t\t(layer "B.Cu" (type "copper") (thickness 0.035))
\t\t\t(layer "B.Mask" (type "Bottom Solder Mask") (thickness 0.01))
\t\t\t(layer "B.Paste" (type "Bottom Solder Paste"))
\t\t\t(layer "B.SilkS" (type "Bottom Silk Screen"))
\t\t\t(dielectric_constraints no)
\t\t)
'''
s = open(pcb).read()
if "(stackup" not in s:
    s = s.replace("\t(setup\n", "\t(setup\n" + STACKUP, 1)
s = s.replace('(4 "In1.Cu" signal)', '(4 "In1.Cu" power "GND")')
s = s.replace('(6 "In2.Cu" signal)', '(6 "In2.Cu" mixed "PWR")')
open(pcb, "w").write(s)

p = json.load(open(pro))
r = p["board"]["design_settings"]["rules"]
r.update({
    "min_clearance": 0.127, "min_track_width": 0.127,
    "min_via_diameter": 0.45, "min_through_hole_diameter": 0.2,
    "min_via_annular_width": 0.15, "min_hole_clearance": 0.15,
    "min_hole_to_hole": 0.4, "min_copper_edge_clearance": 0.3,
    "min_text_height": 0.8, "min_text_thickness": 0.15,
})
# name: (track, clearance, via_dia, via_drill, diff_width, diff_gap)
NC = {
    "Default":              (0.2,  0.15, 0.6, 0.3, None, None),
    "GND":                  (0.4,  0.2,  0.6, 0.3, None, None),
    "PWR_3V3":              (0.4,  0.2,  0.6, 0.3, None, None),
    "PWR_5V":               (0.5,  0.2,  0.6, 0.3, None, None),
    "PWR_BUCK_5V":          (0.5,  0.2,  0.6, 0.3, None, None),
    "PWR_VBUS":             (0.5,  0.2,  0.6, 0.3, None, None),
    "PWR_19V":              (0.4,  0.2,  0.6, 0.3, None, None),
    "MAGJACK_TRANSFORMERS": (0.3,  0.2,  0.6, 0.3, None, None),
    "ETH_Lines":            (0.16, 0.2,  0.6, 0.3, 0.16, 0.15),
    "USB":                  (0.25, 0.2,  0.6, 0.3, 0.25, 0.15),
    "RMII":                 (0.2,  0.15, 0.6, 0.3, None, None),
    "ETH_CTL":              (0.2,  0.15, 0.6, 0.3, None, None),
    "SWD":                  (0.2,  0.15, 0.6, 0.3, None, None),
    "RST":                  (0.2,  0.15, 0.6, 0.3, None, None),
    "IC2_1":                (0.2,  0.15, 0.6, 0.3, None, None),
    "TEMP_DATA":            (0.2,  0.15, 0.6, 0.3, None, None),
    "PWR_DRV_BANK_0":       (0.2,  0.15, 0.6, 0.3, None, None),
    "PWR_DRV_BANK_1":       (0.2,  0.15, 0.6, 0.3, None, None),
    "RST_DRV_BANK_0":       (0.2,  0.15, 0.6, 0.3, None, None),
    "RST_DRV_BANK_1":       (0.2,  0.15, 0.6, 0.3, None, None),
}
for c in p["net_settings"]["classes"]:
    v = NC.get(c["name"])
    if not v:
        print("no values for", c["name"]); continue
    t, cl, vd, vh, dw, dg = v
    c.update({"track_width": t, "clearance": cl, "via_diameter": vd, "via_drill": vh,
              "microvia_diameter": 0.3, "microvia_drill": 0.1})
    if dw:
        c.update({"diff_pair_width": dw, "diff_pair_gap": dg, "diff_pair_via_gap": 0.25})
json.dump(p, open(pro, "w"), indent=2)
open(pro, "a").write("\n")
print("ok")
