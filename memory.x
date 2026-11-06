/* nRF52832-QFAA as used in the PineTime: 512 KiB flash, 64 KiB RAM.
 *
 * We link at 0x0 and so overwrite whatever bootloader was there (the dev kit
 * ships with InfiniTime's MCUBoot). There is no SoftDevice to leave room for.
 */
MEMORY
{
  FLASH : ORIGIN = 0x00000000, LENGTH = 512K
  RAM   : ORIGIN = 0x20000000, LENGTH = 64K
}
