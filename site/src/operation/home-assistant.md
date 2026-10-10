# Home Assistant

The controller publishes retained JSON over MQTT, so Home Assistant's
MQTT integration can show and control a frame with plain YAML. The
example below comes from the firmware repository and is kept there as
`firmware/docs/home-assistant.md`. Replace the site (`default`) and the
device id (`granite-37adc7`) with yours.

{{#include ../../../firmware/docs/home-assistant.md:3:}}
