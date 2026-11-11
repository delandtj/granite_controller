/* STM32G0B1CCT6 on the PP-48 core-fd board, the link-fdcan build
 * (firmware/plane-core-fd-pins.toml, hardware ADR 0006 decision 6):
 * 256 KB flash in 2 KB pages, 144 KB RAM.
 *
 * Picked by build.rs from the link feature; the G070 builds use
 * memory-g070cb.x. Neither file is called memory.x, because the one on
 * the linker's search path has to be the one build.rs copied: ld
 * resolves `INCLUDE memory.x` from the current directory first, so a
 * memory.x in the crate root would quietly win over either of these.
 *
 * This build takes the whole flash, because plane-boot does not exist
 * yet and plane-fd is flashed over SWD at 0x08000000. The slot plan of
 * ADR 0002 decision 2 is the same on this part (the pages are 2 KB
 * here too), with the spare flash left over at the top:
 *
 *   0x08000000  16 KB  BOOT    plane-boot, SWD only, never in the field
 *   0x08004000   2 KB  STATE   embassy-boot swap state
 *   0x08004800   4 KB  CONFIG  2 x 2 KB, sequence number + CRC
 *   0x08005800  52 KB  ACTIVE  the running application  <- plane-fd
 *   0x08012800  54 KB  DFU     update image (ACTIVE + 1 page)
 *   0x08020000 128 KB  spare   bank 2, unused by the ADR's plan
 *
 * Until the bootloader lands the two CONFIG copies live at 0x0801F000
 * and 0x0801F800 (src/config_store.rs): the last two pages of the first
 * 128 KB, which is bank 1 whatever the part's DUAL_BANK option bit says,
 * and far out of reach of a ~50 KB image. They are inside this FLASH
 * region, so an image over 124 KB would reach them - it is nowhere
 * near, and the bootloader moves them to 0x08004800 anyway.
 */
MEMORY
{
  FLASH : ORIGIN = 0x08000000, LENGTH = 256K
  RAM   : ORIGIN = 0x20000000, LENGTH = 144K
}
