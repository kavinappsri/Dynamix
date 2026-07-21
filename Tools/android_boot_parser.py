import struct
import sys

# ==========================
# ANDROID BOOT IMAGE PARSER
# ==========================

# FUNCTIONS ====================

#Extracts given sectors into files
def extract_part(file, offset, size, output_name):
    file.seek(offset)

    data = file.read(size)

    with open(output_name, "wb") as out:
        out.write(data)

    print(f"Extracted {output_name}")

#Returns aligned sector
PAGE = None
def align(x):
    return ((x + PAGE - 1) // PAGE) * PAGE

# =================================

# MAIN ============================

min_args_number = 2
if len(sys.argv) != min_args_number:
    print("ABORT - Missing args")
    print("Usage: python android_boot_parser.py <boot.img>")
    sys.exit(1)

with open(sys.argv[1], 'rb') as f:
    header = f.read(608) #Magic Number -> Header Size | * Remove it in future

# Android boot image check - its specified with the magic header
magic = header[:8]
if magic != b"ANDROID!":
    print("ABORT - Not android boot image")
    sys.exit(1)

print("Android boot image detected")

# Android boot image components listed here as
# unsigned 32 bit little endian integers
# < -> Little Endian
# I -> unsigned 32 bit Integers

kernel_size = struct.unpack("<I", header[8:12])[0]
kernel_addr = struct.unpack("<I", header[12:16])[0]
ramdisk_size = struct.unpack("<I", header[16:20])[0]
ramdisk_addr = struct.unpack("<I", header[20:24])[0]
second_size = struct.unpack("<I", header[24:28])[0]
second_addr = struct.unpack("<I", header[28:32])[0]
page_size = struct.unpack("<I", header[36:40])[0]

# android cmdline args

cmdline = (
    header[64:576]
    .split(b"\0", 1)[0]
    .decode("ascii", errors="ignore")
)

# Sector Extraction
PAGE = page_size

kernel_offset = PAGE

ramdisk_offset = kernel_offset + align(kernel_size)

second_offset = ramdisk_offset + align(ramdisk_size)

with open(sys.argv[1], "rb") as f:

    extract_part(f, kernel_offset, kernel_size, "kernel.bin")

    extract_part(f, ramdisk_offset, ramdisk_size, "ramdisk.gz")

    if second_size > 0:
        extract_part(f, second_offset, second_size, "second.bin")

#print to output if main
def main():
    print("Kernel Size :", kernel_size)
    print("Kernel Addr :", hex(kernel_addr))

    print()

    print("Ramdisk Size:", ramdisk_size)
    print("Ramdisk Addr:", hex(ramdisk_addr))

    print()

    print("Second Size :", second_size)
    print("Second Addr :", hex(second_addr))

    print()

    print("Page Size   :", page_size)

    print()

    print("Cmdline:")
    print(cmdline)

if __name__ == "__main__":
    main()
    
