"""Module input ripple through pp-48-unit-fd to the 48 V cable: a harmonic model, 2026-11-11.

Run: uv run --no-project --with numpy python -I unit_ripple.py
Source of the predicted numbers in docs/unit-fd-emi-bench-plan.md. A model, not a measurement: every
value below is an assumption to be replaced by what the bench shows.

Circuit, per harmonic k of fsw (nodal analysis, AC only):

    module (current source, CMOD) -- M --[slot: shunts + Q1]-- B --[unit: shunts + Q202||Q203]-- V --[cable]-- supply
                                      |                         |                                 |
                                     C2 (behind the standoff)  C214 + C215 (+ extra)            C202, C201

- M: module input = slot OUT (A_OUT / B_OUT). B: the VBUS plane. V: VIN at the XT60. The supply end of
  the cable is an AC short (worst case for cable current).
- The module is a buck drawing a rectangular current pulse of IOUT for D = VOUT / VIN of each period
  (inductor ripple ignored); its harmonics are injected at M.
- n_mod identical modules switching in phase (the envelope maximum of two unsynchronised modules), so
  M is one node by symmetry.
- Ceramics at their 48 V bias value: docs/pm-19v-250w-b.md gives 1.9 uF kept per 4.7 uF / 100 V 1210,
  the same ratio is applied to the unit's 2.2 uF / 100 V 1210 (0.9 uF).
- C201 (Lelon RVT2A470M1010, 47 uF / 100 V): no 100 kHz ESR found, 0.4 ohm assumed, swept below.
- Path resistances from the schematic (R1||R2 3.5 mohm + Q1 7 mohm; R201||R202 3 mohm + Q202||Q203
  2 mohm); path and part inductances are estimates of a few nH and barely matter at these frequencies.
"""
import numpy as np

FSW = 252e3          # PM-19V-250W-B
IOUT = 13.2          # A, module output at full load
VIN = 48.0
VOUT = 19.0
HARMONICS = 40


def cap(c, esr, esl):
    """Impedance of a capacitor with series resistance and inductance."""
    return lambda w: esr + 1j * w * esl + 1 / (1j * w * c)


def run(label, n_mod=2, cbus_extra=0.0, lslot_extra=0.0, lin_extra=0.0, lcab=0.5e-6, esr201=0.4):
    """Solve the network per harmonic; print the 252 kHz peaks and the rms totals; return the rows."""
    duty = VOUT / VIN
    z_mod = cap(11e-6, 2e-3, 0.3e-9)                                   # 6 x 4.7 uF on the module
    z_c2 = lambda w: 1j * w * 3e-9 + cap(0.9e-6, 5e-3, 1e-9)(w)        # C2 / C52, 3 nH of standoff
    z_slot = lambda w: 10.5e-3 + 1j * w * (6e-9 + lslot_extra)         # slot shunts + Q1 + pours / vias
    z_bus = cap(1.8e-6, 3e-3, 0.6e-9)                                  # C214 + C215
    z_unit = lambda w: 5e-3 + 1j * w * 4e-9                            # unit shunts + Q202 || Q203
    z_c202 = cap(0.9e-6, 5e-3, 1e-9)
    z_c201 = cap(47e-6, esr201, 6e-9)
    z_cab = lambda w: 15e-3 + 1j * w * (lcab + lin_extra)

    sq = dict(M=0.0, B=0.0, V=0.0, cable=0.0, c201=0.0, shunt=0.0)
    rows = []
    for k in range(1, HARMONICS + 1):
        w = 2 * np.pi * FSW * k
        i_k = 2 * IOUT * np.sin(np.pi * k * duty) / (np.pi * k)       # peak of harmonic k of the pulse
        y_m = 1 / z_mod(w) + 1 / z_c2(w)
        y_s = 1 / z_slot(w)
        y_b = 1 / z_bus(w)
        if cbus_extra:
            y_b += 1 / cap(cbus_extra, 4e-3, 0.5e-9)(w)
        y_u = 1 / z_unit(w)
        y_v = 1 / z_c202(w) + 1 / z_c201(w) + 1 / z_cab(w)
        a = np.array([[y_m + y_s, -y_s, 0],
                      [-n_mod * y_s, n_mod * y_s + y_b + y_u, -y_u],
                      [0, -y_u, y_u + y_v]], dtype=complex)
        m, b, v = np.linalg.solve(a, np.array([i_k, 0, 0], dtype=complex))
        i_cab, i_c201, i_shunt = v / z_cab(w), v / z_c201(w), (m - b) / z_slot(w)
        for key, val in (("M", m), ("B", b), ("V", v), ("cable", i_cab), ("c201", i_c201), ("shunt", i_shunt)):
            sq[key] += abs(val) ** 2 / 2
        rows.append((k, abs(m), abs(b), abs(v), abs(i_cab)))
    rms = {key: np.sqrt(val) for key, val in sq.items()}
    _, m1, b1, v1, _ = rows[0]
    print(f"{label:50s} 252 kHz pk: OUT {m1 * 1e3:4.0f}  VBUS {b1 * 1e3:4.0f}  XT60 {v1 * 1e3:4.0f} mV | "
          f"rms: VBUS {rms['B'] * 1e3:4.0f} mV  cable {rms['cable']:4.2f} A  C201 {rms['c201']:4.2f} A  "
          f"slot shunt {rms['shunt']:4.2f} A")
    return rows


if __name__ == "__main__":
    print("modules in phase, 0.5 uH cable, C201 ESR 0.4 ohm unless noted")
    run("as built, 1 module", n_mod=1)
    as_built = run("as built, 2 modules")
    run("as built, 0.2 uH cable (short / busbar)", lcab=0.2e-6)
    run("as built, 2 uH cable", lcab=2e-6)
    for esr in (0.2, 1.0, 2.0):
        run(f"as built, C201 ESR {esr} ohm", esr201=esr)
    run("+ 4 x 2.2 uF on VBUS (2 per slot)", cbus_extra=3.6e-6)
    run("+ 10 x 2.2 uF on VBUS", cbus_extra=9e-6)
    run("+ 33 uF low-ESR on VBUS", cbus_extra=33e-6)
    run("+ 1 uH at the unit input (cable side of C201)", lin_extra=1e-6)
    run("+ 1 uH per slot (between Q1 and C2)", lslot_extra=1e-6)
    run("+ 1 uH per slot + 4 x 2.2 uF on VBUS", lslot_extra=1e-6, cbus_extra=3.6e-6)
    print("harmonics as built, 2 modules: k, OUT, VBUS, XT60 (mV pk), cable (A pk)")
    for k, m, b, v, i_cab in as_built[:8]:
        print(f"  {k} {m * 1e3:6.0f} {b * 1e3:6.0f} {v * 1e3:6.0f} {i_cab:6.2f}")
