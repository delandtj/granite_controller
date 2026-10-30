# 3D models

Models for parts the KiCad 3D library does not ship. Footprints reference
them as `${KIPRJMOD}/3dmodels/<file>`.

| File | Part | Used by | Source |
|---|---|---|---|
| TLP176AM_SO4.step | Toshiba TLP176AM (LCSC C5370540) | K1-K16 | EasyEDA/LCSC via easyeda2kicad |
| TLP290-4_SOP16.step | Toshiba TLP290-4 (LCSC C39031) | U16, U17 | EasyEDA/LCSC via easyeda2kicad |
| HR911105A.step | HanRun HR911105A (LCSC C12074) | J1 | EasyEDA/LCSC via easyeda2kicad |
| HRO_TYPE-C-31-M-12.step | Korean Hroparts TYPE-C-31-M-12 (LCSC C165948) | J2 | EasyEDA/LCSC via easyeda2kicad |
| ESP32-C6-WROOM-1.step | Espressif ESP32-C6-WROOM-1 | U1 | github.com/espressif/kicad-libraries, CC-BY-SA 4.0 with the KiCad library design exception |

Placement on our footprints (offset in mm, KiCad 3D axes, Y up; rotation
about Z), found by matching the source footprint's pads to ours and
checked in a 3D render (pin-1 dot on the silk pin-1 mark). The SOP-16
needed 90, not the 270 the pad match suggested.

| Footprint | Offset | Rotation |
|---|---|---|
| Package_SO:SOIC-4_4.55x3.7mm_P2.54mm | 0, 0, 0 | 0 |
| Package_SO:SOP-16_4.55x10.3mm_P1.27mm | 0, 0, 0 | 90 |
| Connector_RJ:RJ45_Hanrun_HR911105A_Horizontal | 4.443, -4.357, 0 | 0 |
| granite:ESP32-C6-WROOM-1 | -9, -12.75, 0 | 0 |
| Connector_USB:USB_C_Receptacle_HRO_TYPE-C-31-M-12 | 0, 1.424, 0 | 180 |

The four stock-library footprints carry the model on the board only: an
"Update footprints from library" in KiCad drops it again. The ESP32 model
is also set in footprints/granite.pretty.
