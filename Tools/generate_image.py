import sys
import os
from PIL import Image

def main():
    if len(sys.argv) < 3:
        print("Usage: python3 generate_logo.py <input_png_path> <output_dir_or_file>")
        sys.exit(1)

    input_png_path = sys.argv[1]
    output_path = sys.argv[2]

    if os.path.isdir(output_path):
        output_path = os.path.join(output_path, "logo.bin") # Use .bin extension

    img = Image.open(input_png_path).convert("RGB")
    width, height = img.size
    print(f"[+] Processing {width}x{height} raw image to binary...")

    # Write raw little-endian bytes directly
    with open(output_path, "wb") as f:
        for y in range(height):
            for x in range(width):
                r, g, b = img.getpixel((x, y))
                # 4 bytes per pixel: B, G, R, X (Little-endian layout for u32 0x00RRGGBB)
                f.write(bytes([b, g, r, 0x00]))

    print(f"[+] Success! Raw binary written to: {output_path}")

if __name__ == "__main__":
    main()
