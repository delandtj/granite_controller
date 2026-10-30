# Immersion constraints

The controller and the power planes are built to run submerged in
single-phase hydrocarbon fluid, with two-phase fluorinated fluid as a
possible later case. The boards themselves follow these rules; the
cabling, glands, port module screws and anything you add on J9 have to
follow them as well.

## Electrical safety in the fluid

- **No fuses in the fluid.** Fuses are rated in air: cooled by the
  fluid they blow late or not at all, they can arc in it, and they
  cannot be replaced in the tank. Overcurrent inside the tank is
  handled by electronic limiters (one hot-swap per unit, one per slot,
  the port module's own current limit). The only fuse or breaker is
  **outside the fluid, per box or per branch** of the 48 V bus.
- The bus is SELV: 48-60 V nominal, and every plane part on it is rated
  100 V. Bus transients are clamped by a TVS at every board input.
- The controller side (controller, Ethernet, USB, probes, the link and
  12 V aux cables) is isolated from the bus on each plane. This is
  functional isolation, not a safety barrier; keep it intact by never
  connecting the controller's ground to the bus ground.
- Reverse polarity on the bus is prevented only by the keyed XT30 /
  XT60 connectors. Make and check the power cables before they go into
  the tank.

## Materials

- No PVC cable jackets. Use PE, PTFE, FEP or ETFE insulated wire.
- No silicone in the tank unless it has been soak-tested in your fluid:
  not as jacket, potting, thermal pad or LED lens.
- Connector housings in PA, PBT or LCP. JST PH and GH housings are PA;
  their long-term behaviour in the target fluid is one of the open
  soak-test items and should be verified for a production run.
- No adhesive labels on the boards: they come off and foul the pump.
  Record MACs, plane addresses and serials elsewhere; the silkscreen is
  cosmetic.
- No parts with vents or sealed air cavities, on the boards or on
  add-ons, and no fans or heatsinks anywhere.

## Connectors in the fluid

- Every plane joint is a JST PH 14-pin chain cable: tin-plated 2.0 mm
  contacts with a friction lock, no positive latch. Seat each housing
  fully and secure the cable (a strain relief tie through a board hole)
  so it cannot work loose. A chain cable that comes off leaves the
  units behind it on their hardware defaults (slots on, no control).
- The port module's power runs through its four M2 screws; use the
  split lock washers so the clamping holds through thermal cycles.

## Cables through the tank wall

- Oil wicks along stranded conductors. Use solid-conductor cables, or
  potted glands where a stranded cable leaves the fluid.
- The Ethernet pairs on the controller are impedance-calculated for the
  fluid (relative permittivity about 2.1), so the Ethernet cable may run
  through the fluid to a gland in the wall.
- The USB-C service port is calculated for air. Use it with the board
  out of the fluid, before sealing and after a board has been lifted
  out for service.

## Things you lose once sealed

- The controller's USB console. After sealing, the only paths into the
  controller are the network and, in the last resort, lifting the board
  out.
- The plane cores' SWD connectors and their address DIP switches. A
  plane's address cannot be changed once it is in the fluid.
- Pressing BOOT or EN on the controller: there are no buttons, only
  test pads.
- The status LEDs, the plane core LEDs and the RJ45 LEDs as a reliable
  indication: they are there, but do not plan a procedure around
  seeing them.

The commissioning chapters are written around these losses: every
network-affecting change is commit-confirmed, controller firmware
updates validate themselves and roll back, plane firmware is to be
updated from the controller over the link, and every controller has a recovery
token and optionally a fleet key so it can be reset to factory state
from the network. Record the MAC and token of every controller and the
address of every plane **before** they go into the fluid.

## Soak testing

The OCP immersion component compatibility guideline asks for at least
500 hours at about 1.2 times the operating temperature for any part of
uncertain material. Open items at the time of writing: on the
controller the HanRun magjack (LED lenses and potting), the JST
housings and the ESP32 module's shield can; on the planes the JST PH
and GH housings and contacts, the XT30 / XT60 connectors, the screw
terminals, the DIP switch, the photoMOS and isolator packages, and the
port module's aux header and socket. Add your probes, floats and cable
to that list for your fluid.
