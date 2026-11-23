/*
 * SPCR (Serial Port Console Redirection) for QEMU q35: tells the guest that
 * the firmware console is the 16550 at I/O port 0x3F8.
 *
 * q35 has no SPCR of its own, so without this the kernel just uses the first
 * serial port ACPI lists (COM2 when there are two).
 *
 * Compile:  iasl qemu-q35-spcr.asl           (writes qemu-q35-spcr.aml)
 * Attach:   qemu-system-x86_64 ... -acpitable file=qemu-q35-spcr.aml
 *
 * Layout is SPCR revision 4, the one current iasl versions expect.
 *
 * COM1 is 0x3F8 / IRQ 4 and COM2 is 0x2F8 / IRQ 3. To point at the other
 * one, change "Address", "PCAT-compatible IRQ" and "Interrupt".
 */
[0004]                          Signature : "SPCR"    [Serial Port Console Redirection Table]
[0004]                       Table Length : 0000005A
[0001]                           Revision : 04
[0001]                           Checksum : 00
[0006]                             Oem ID : "DYNMX "
[0008]                       Oem Table ID : "QEMUSPCR"
[0004]                       Oem Revision : 00000001
[0004]                    Asl Compiler ID : "INTL"
[0004]              Asl Compiler Revision : 20260930

[0001]                     Interface Type : 00
[0003]                           Reserved : 000000

[0012]               Serial Port Register : [Generic Address Structure]
[0001]                           Space ID : 01 [SystemIO]
[0001]                          Bit Width : 08
[0001]                         Bit Offset : 00
[0001]               Encoded Access Width : 01 [Byte Access]
[0008]                            Address : 00000000000003F8

[0001]                     Interrupt Type : 01
[0001]                PCAT-compatible IRQ : 04
[0004]                          Interrupt : 00000004
[0001]                          Baud Rate : 07
[0001]                             Parity : 00
[0001]                          Stop Bits : 01
[0001]                       Flow Control : 00
[0001]                      Terminal Type : 00
[0001]                           Language : 00
[0002]                      PCI Device ID : FFFF
[0002]                      PCI Vendor ID : FFFF
[0001]                            PCI Bus : 00
[0001]                         PCI Device : 00
[0001]                       PCI Function : 00
[0004]                          PCI Flags : 00000000
[0001]                        PCI Segment : 00
[0004]                    Uart Clock Freq : 00000000
[0004]                  Precise Baud rate : 00000000
[0002]              NameSpaceStringLength : 0002
[0002]              NameSpaceStringOffset : 0058
[0002]                    NamespaceString : "."
