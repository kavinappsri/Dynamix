from PIL import Image
from pathlib import Path
from sys import exit

#THIS IS CURRENTLY HARDCODED TO BOOT IMAGE
script_dir = Path(__file__).resolve().parent
boot_img_path = script_dir.parent / "Assets" / "boot_logo.png"

if not boot_img_path.exists():
    print("ABORT - Image not found")
    exit()

image = Image.open(boot_img_path)
image = image.convert("RGB")
width, height = image.size

print("Image found")

# CONSTANTS
drw_magic = b"DRW0"
drw_version = bytes([1]) #v1
data_type = bytes([1]) # data type 1 - image
header_length = (12).to_bytes(2, "little")
width_bytes = width.to_bytes(4, "little")
height_bytes = height.to_bytes(4, "little")
pixel_format = bytes([1])
r_byte = bytes([0])
r_u16 = (0).to_bytes(2, "little")

output_path = script_dir.parent / "Assets" / "boot_logo.drw"

with open(output_path, "wb") as output:
    print("File Created")

    output.write(drw_magic)
    output.write(drw_version)
    output.write(data_type)
    output.write(header_length)
    output.write(width_bytes)
    output.write(height_bytes)
    output.write(pixel_format)
    output.write(r_byte)
    output.write(r_u16)

    print("Headers Written")

    for r, g, b in image.getdata():
        r_5 = r >> 3
        g_6 = g >> 2
        b_5 = b >> 3

        rgb565 = (r_5 << 11) | (g_6 << 5) | b_5

        output.write(rgb565.to_bytes(2, "little"))

    print("Pixels Written")

print("DRW written successfully.")
print(f"Resolution : {width} x {height}")
print(f"Pixels     : {width * height}")
print(f"Output     : {output_path}")
