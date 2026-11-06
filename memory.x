/* nRF52832-QFAA as used in the PineTime: 512 KiB flash, 64 KiB RAM.
 *
 * We link at 0x0 and so overwrite whatever bootloader was there (the dev kit
 * ships with InfiniTime's MCUBoot). There is no SoftDevice to leave room for.
 *
 * The last 16 KiB (four 4 KiB pages) hold persistent storage (see
 * src/storage.rs, whose RANGE must match). Keeping it out of FLASH means the
 * firmware can never grow into it.
 */
MEMORY
{
  FLASH   : ORIGIN = 0x00000000, LENGTH = 512K - 16K
  STORAGE : ORIGIN = 0x0007C000, LENGTH = 16K
  RAM     : ORIGIN = 0x20000000, LENGTH = 64K
}
