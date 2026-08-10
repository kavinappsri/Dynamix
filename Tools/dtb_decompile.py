import struct
import sys

FDT_MAGIC = 0xD00DFEED
FDT_BEGIN_NODE = 1
FDT_END_NODE = 2
FDT_PROP = 3
FDT_NOP = 4
FDT_END = 9

def decompile_fdt(data, off_struct, off_strings, size_struct):
    def get_string(offset):
        end = data.find(b'\x00', off_strings + offset)
        if end == -1:
            return ""
        return data[off_strings + offset:end].decode('ascii', errors='ignore')

    offset = off_struct
    indent = 0

    print("// ==========================================")
    print("// Decompiled Device Tree Blob (DTS)")
    print("// ==========================================\n")
    print("/dts-v1/;\n")

    while offset < off_struct + size_struct:
        token = struct.unpack(">I", data[offset:offset+4])[0]
        offset += 4

        if token == FDT_BEGIN_NODE:
            end = data.find(b'\x00', offset)
            name = data[offset:end].decode('ascii', errors='ignore')
            offset = end + 1
            offset = (offset + 3) & ~3  # Align to 4 bytes

            node_name = name if name else "/"
            print("  " * indent + f"{node_name} {{")
            indent += 1

        elif token == FDT_END_NODE:
            indent = max(0, indent - 1)
            print("  " * indent + "};")

        elif token == FDT_PROP:
            len_val, nameoff = struct.unpack(">2I", data[offset:offset+8])
            offset += 8
            prop_name = get_string(nameoff)
            prop_val = data[offset:offset+len_val]
            offset += len_val
            offset = (offset + 3) & ~3  # Align to 4 bytes

            # Format property value
            if len_val == 0:
                val_str = ";"
            elif all(32 <= b <= 126 for b in prop_val[:-1]) and prop_val.endswith(b'\x00') and len_val > 1:
                # String value
                strs = [s.decode('ascii', errors='ignore') for s in prop_val.rstrip(b'\x00').split(b'\x00')]
                formatted = ", ".join(f'"{s}"' for s in strs)
                val_str = f" = {formatted};"
            elif len_val % 4 == 0:
                # Array of 32-bit cells (integers/addresses)
                words = struct.unpack(f">{len_val//4}I", prop_val)
                hex_words = " ".join(f"0x{w:x}" for w in words)
                val_str = f" = <{hex_words}>;"
            else:
                # Byte array
                hex_bytes = " ".join(f"{b:02x}" for b in prop_val)
                val_str = f" = [{hex_bytes}];"

            print("  " * indent + f"{prop_name}{val_str}")

        elif token == FDT_NOP:
            continue

        elif token == FDT_END:
            break

def main():
    if len(sys.argv) < 2:
        print("Usage: python dtb_decompile.py <dump_file.img>")
        sys.exit(1)

    with open(sys.argv[1], "rb") as f:
        data = f.read()

    # Search for FDT Magic Number (0xD00DFEED)
    magic_bytes = b"\xd0\x0d\xfe\xed"
    pos = data.find(magic_bytes)

    if pos == -1:
        print("Error: No Device Tree (0xD00DFEED) magic header found in file.")
        sys.exit(1)

    print(f"[+] Found DTB header at byte offset: {hex(pos)}")

    # Parse Header
    header = data[pos:pos+40]
    (magic, totalsize, off_struct, off_strings, off_mem_rsvmap,
     version, last_comp_version, boot_cpuid_phys,
     size_strings, size_struct) = struct.unpack(">10I", header)

    print(f"[+] Total DTB Size: {totalsize} bytes")
    print(f"[+] DTB Version: {version}\n")

    # Slice raw DTB
    dtb_data = data[pos : pos + totalsize]

    # Run Decompiler
    decompile_fdt(dtb_data, off_struct, off_strings, size_struct)

if __name__ == "__main__":
    main()