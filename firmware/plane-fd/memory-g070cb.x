/* STM32G070CBT6 on the PP-48 core-fd board, the link-mcp build
 * (firmware/plane-core-fd-pins.toml):
 * 128 KB flash in 2 KB pages, 36 KB RAM, no CCM, no backup SRAM.
 *
 * Picked by build.rs from the link feature; link-fdcan uses
 * memory-g0b1cc.x. Neither file is called memory.x, because the one on
 * the linker's search path has to be the one build.rs copied: ld
 * resolves `INCLUDE memory.x` from the current directory first, so a
 * memory.x in the crate root would quietly win over either of these.
 *
 * This build takes the whole flash, because plane-boot does not exist
 * yet and plane-fd is flashed over SWD at 0x08000000. The slot plan of
 * ADR 0002 decision 2, which this file has to grow into once the
 * bootloader lands, is:
 *
 *   0x08000000  16 KB  BOOT    plane-boot, SWD only, never in the field
 *   0x08004000   2 KB  STATE   embassy-boot swap state
 *   0x08004800   4 KB  CONFIG  2 x 2 KB, sequence number + CRC
 *   0x08005800  52 KB  ACTIVE  the running application  <- plane-fd
 *   0x08012800  54 KB  DFU     update image (ACTIVE + 1 page)
 *
 * So the application has to fit in ~52 KB (ADR 0002 "Code size" risk:
 * drop to a copy-only update above ~48 KB). Until then the two CONFIG
 * copies live in the last two pages of flash (0x0801F000 and
 * 0x0801F800, src/config_store.rs), which the image cannot reach while
 * it is well under 124 KB; they move to 0x08004800 with the bootloader.
 */
MEMORY
{
  FLASH : ORIGIN = 0x08000000, LENGTH = 128K
  RAM   : ORIGIN = 0x20000000, LENGTH = 36K
}
