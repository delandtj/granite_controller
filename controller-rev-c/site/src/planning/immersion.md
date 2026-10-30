# Immersion constraints

The controller is built to run submerged in single-phase hydrocarbon
fluid, with two-phase fluorinated fluid as a possible later case. The
board itself follows these rules; the cabling, glands and anything you
add on J9 or J8 has to follow them as well.

## Materials

- No PVC cable jackets. Use PE, PTFE, FEP or ETFE insulated wire.
- No silicone in the tank unless it has been soak-tested in your fluid:
  not as jacket, potting, thermal pad or LED lens.
- Connector housings in PA, PBT or LCP. JST PH housings are PA; their
  long-term behaviour in the target fluid is one of the open soak-test
  items and should be verified for a production run.
- No adhesive labels on the board: they come off and foul the pump.
  Record the MAC and serial elsewhere; the silkscreen is cosmetic.
- No parts with vents or sealed air cavities, on the board or on
  add-ons. The board uses MLCC and molded tantalum-polymer capacitors
  for that reason.

## Cables through the tank wall

- Oil wicks along stranded conductors. Use solid-conductor cables, or
  potted glands where a stranded cable leaves the fluid.
- The Ethernet pairs on the board are impedance-calculated for the
  fluid (relative permittivity about 2.1), so the Ethernet cable may run
  through the fluid to a gland in the wall.
- The USB-C service port is calculated for air. Use it with the board
  out of the fluid, before sealing and after a board has been lifted
  out for service.

## Things you lose once sealed

- The USB console. After sealing, the only paths into the board are the
  network and, in the last resort, lifting the board out.
- Pressing BOOT or EN: there are no buttons, only test pads.
- The status LED and the RJ45 LEDs as a reliable indication: they are
  there, but do not plan a procedure around seeing them.

The commissioning chapters are written around these losses: every
network-affecting change is commit-confirmed, firmware updates validate
themselves and roll back, and every board has a recovery token and
optionally a fleet key so it can be reset to factory state from the
network. Record the MAC and token of every board **before** it goes
into the fluid.

## Soak testing

The OCP immersion component compatibility guideline asks for at least
500 hours at about 1.2 times the operating temperature for any part of
uncertain material. Open items on the board itself at the time of
writing are the HanRun magjack (LED lenses and potting), the JST
housings, the J8 header body and the ESP32 module's shield can. Add
your probes, floats and cable to that list for your fluid.
