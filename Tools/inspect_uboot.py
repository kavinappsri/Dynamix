import struct
import sys
import re

# Known physical MMIO register bases for Rockchip RK312x SoCs
PERIPHERAL_MMIO_MAP = {
    "LCDC / VOP (Display Controller)": 0x1010E000,
    "GIC Distributor (Interrupts)":    0x10138000,
    "GIC CPU Interface":               0x1013A000,
    "SD / MMC Controller (dw_mmc)":    0x10214000,
    "CRU (Clock & Reset Unit)":        0x20060000,
    "UART0 (Serial)":                  0x20060000,
    "UART1 (Serial)":                  0x20064000,
    "UART2 (Debug Console)":           0x20068000,
    "Timer 0 (Hardware Timer)":        0x20044000,
    "I2C0 Controller":                 0x20072000,
    "I2C1 Controller (PMIC Bus)":      0x20056000,
    "PWM Controller":                  0x20050000,
    "GRF (General Register Files)":    0x20008000,
    "SRAM (Internal Memory)":          0x10080000,
}

DRIVER_CATEGORIES = {
    "Power & PMIC":      [b"rk816", b"rk808", b"pmic", b"regulator", b"ldo", b"i2c"],
    "Clocks & Reset":    [b"rk_cls", b"cru", b"gpll", b"cpll", b"dpll", b"clk_rk312x"],
    "Storage & NAND":    [b"rknand", b"rk_nand", b"sdmmc", b"dw_mmc", b"ftl"],
    "Display & Output":  [b"lcdc", b"rk312x_lcdc", b"vop", b"bmp", b"backlight", b"panel"],
    "USB & Fastboot":    [b"rockusb", b"fastboot", b"dwc2", b"usb_phy", b"gadget"],
    "Security & Trust":  [b"psci", b"atf", b"trust", b"optee", b"fit"],
}

def scan_mmio_addresses(data):
    print("==================================================")
    print(" 1. HARDWARE MMIO REGISTER REFERENCES DETECTED")
    print("==================================================")
    for name, addr in PERIPHERAL_MMIO_MAP.items():
        addr_bytes = struct.pack("<I", addr) # ARM 32-bit Little-Endian
        count = data.count(addr_bytes)
        if count > 0:
            print(f"  [✓] {name:<35} @ 0x{addr:08X} (Referenced {count}x)")
        else:
            print(f"  [ ] {name:<35} @ 0x{addr:08X} (Not directly hardcoded)")

def scan_driver_subsystems(data):
    print("\n==================================================")
    print(" 2. INCLUDED SUBSYSTEMS & DRIVERS")
    print("==================================================")
    data_lower = data.lower()
    for cat, keywords in DRIVER_CATEGORIES.items():
        print(f"\n  [{cat}]")
        for kw in keywords:
            count = data_lower.count(kw)
            status = f"Present ({count} matches)" if count > 0 else "Missing"
            print(f"    - {kw.decode():<16}: {status}")

def extract_env_and_cmdline(data):
    print("\n==================================================")
    print(" 3. U-BOOT CONFIGURATION & BOOT ARGS")
    print("==================================================")

    # Search for standard ASCII printable sequences
    strings = re.findall(b"[ -~]{10,}", data)

    boot_vars = []
    cmdlines = []

    for s in strings:
        text = s.decode('ascii', errors='ignore')
        if any(eq in text for eq in ["bootargs=", "bootcmd=", "bootdelay=", "fdt_addr=", "kernel_addr="]):
            boot_vars.append(text)
        elif "console=" in text or "androidboot." in text:
            cmdlines.append(text)

    if boot_vars:
        print("  Found Environment Variables:")
        for var in set(boot_vars[:10]):
            print(f"    * {var}")
    else:
        print("  No plaintext environment variables found (may be compiled statically).")

    if cmdlines:
        print("\n  Default Kernel Commandline Candidates:")
        for cmd in set(cmdlines[:5]):
            print(f"    * {cmd}")

def extract_arm_ram_locations(data):
    print("\n==================================================")
    print(" 4. DETECTED RAM MEMORY ADDRESSES")
    print("==================================================")
    # Search for typical DRAM base addresses (0x60000000 range for RK312x)
    ram_addresses = set()
    for i in range(0, len(data) - 4, 4):
        val = struct.unpack("<I", data[i:i+4])[0]
        if 0x60000000 <= val <= 0x6FFFFFFF and val % 0x100 == 0:
            ram_addresses.add(val)

    sorted_addrs = sorted(list(ram_addresses))
    print(f"  Found {len(sorted_addrs)} potential RAM addresses/pointers in binary.")
    print("  Key aligned boundaries detected:")
    for addr in sorted_addrs:
        if addr in [0x60000000, 0x60000800, 0x60408000, 0x61000000, 0x62000000, 0x68000000]:
            label = ""
            if addr == 0x60000000: label = "(DRAM Physical Base)"
            elif addr == 0x60000800: label = "(DTB / ATAGS Target)"
            elif addr == 0x60408000: label = "(Kernel Load Entry Point)"
            elif addr == 0x62000000: label = "(Ramdisk / Initrd Address)"
            print(f"    * 0x{addr:08X} {label}")

def main():
    filepath = sys.argv[1] if len(sys.argv) > 1 else "uboot.img"
    try:
        with open(filepath, "rb") as f:
            data = f.read()
    except FileNotFoundError:
        print(f"Error: File '{filepath}' not found.")
        sys.exit(1)

    print(f"Analyzing '{filepath}' ({len(data)} bytes)...")
    scan_mmio_addresses(data)
    scan_driver_subsystems(data)
    extract_arm_ram_locations(data)
    extract_env_and_cmdline(data)

if __name__ == "__main__":
    main()