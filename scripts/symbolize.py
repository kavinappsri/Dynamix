"""Turn the addresses in a Dynamix fault report into function names.

Usage:
    scripts/symbolize.py <target-or-file> [logfile]

<target-or-file> is one of:
    aarch64   the newest target/aarch64-unknown-none/{release,debug}/dynamix_kernel
    armv7     the newest target/armv7a-none-eabi/{release,debug}/dynamix_kernel
    x64       target/dynamix_x64.map (written by the linker on every x86_64 build)
    <path>    an ELF kernel image, or a linker .map file

The log is read from `logfile`, or from stdin, so a report can be pasted or
piped in straight from the serial console:

    cargo run ... | tee serial.log          # then, after a crash:
    scripts/symbolize.py aarch64 serial.log
    pbpaste | scripts/symbolize.py x64      # macOS: report copied from the terminal

The symbols must come from the same build that produced the log.

How it works: every report ends with an `anchor:` line, the run-time address of
the function `dynamix_fault_anchor`. Comparing that with the anchor's address
in the symbol file gives the load slide, which is subtracted from every
backtrace address before the lookup.

Optional extras (all auto-detected):
    rust-demangler   pip install rust-demangler    (else rustfilt, llvm-cxxfilt, c++filt)
    file:line info   llvm-symbolizer or addr2line on PATH (ELF images only)
"""

import argparse
import bisect
import glob
import os
import re
import shutil
import struct
import subprocess
import sys

ANCHOR_SYMBOL = "dynamix_fault_anchor"
ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))

SHORTHANDS = {
    "aarch64": ["target/aarch64-unknown-none/{}/dynamix_kernel"],
    "armv7": ["target/armv7a-none-eabi/{}/dynamix_kernel"],
}
X64_MAP = "target/dynamix_x64.map"


# ----------------------------------------------------------------------------
# Symbol tables
# ----------------------------------------------------------------------------

class Symbols:
    """Code symbols sorted by address, with nearest-preceding lookup."""

    def __init__(self, entries):
        # entries: (address, size, name); size 0 means unknown
        entries = sorted(set(entries))
        self.entries = entries
        self.addresses = [e[0] for e in entries]
        self.by_name = {e[2]: e[0] for e in entries}

    def lookup(self, address):
        """Returns (name, offset) of the symbol containing `address`, or None."""
        i = bisect.bisect_right(self.addresses, address) - 1
        if i < 0:
            return None
        start, size, name = self.entries[i]
        offset = address - start
        limit = size if size else 0x10000  # no size known: trust 64 KiB at most
        if offset >= max(limit, 1):
            return None
        return name, offset


def read_elf(path):
    """Executable symbols of an ELF32/ELF64 little-endian image."""
    with open(path, "rb") as f:
        data = f.read()
    if data[:4] != b"\x7fELF":
        raise ValueError("%s is not an ELF file" % path)
    is64 = data[4] == 2
    if data[5] != 1:
        raise ValueError("only little-endian ELF images are supported")

    if is64:
        shoff, = struct.unpack_from("<Q", data, 0x28)
        shentsize, shnum = struct.unpack_from("<HH", data, 0x3A)
        section = lambda i: struct.unpack_from("<IIQQQQIIQQ", data, shoff + i * shentsize)
        # name, type, flags, addr, offset, size, link, info, addralign, entsize
        sym_fmt, sym_size = "<IBBHQQ", 24
    else:
        shoff, = struct.unpack_from("<I", data, 0x20)
        shentsize, shnum = struct.unpack_from("<HH", data, 0x2E)
        section = lambda i: struct.unpack_from("<IIIIIIIIII", data, shoff + i * shentsize)
        sym_fmt, sym_size = "<IIIBBH", 16

    sections = [section(i) for i in range(shnum)]
    SHT_SYMTAB, SHF_EXECINSTR = 2, 4

    entries = []
    for (_, kind, _, _, offset, size, link, _, _, _) in sections:
        if kind != SHT_SYMTAB:
            continue
        str_off = sections[link][4]
        for at in range(offset, offset + size, sym_size):
            if is64:
                name_off, info, _, shndx, value, sym_len = struct.unpack_from(sym_fmt, data, at)
            else:
                name_off, value, sym_len, info, _, shndx = struct.unpack_from(sym_fmt, data, at)
            if info & 0xF != 2 or shndx == 0 or shndx >= len(sections):  # FUNC only
                continue
            if not sections[shndx][2] & SHF_EXECINSTR:
                continue
            end = data.index(b"\0", str_off + name_off)
            name = data[str_off + name_off:end].decode("utf-8", "replace")
            entries.append((value & ~1, sym_len, name))  # clear the Thumb bit
    if not entries:
        raise ValueError("%s has no function symbols (stripped?)" % path)
    return Symbols(entries)


MAP_SECTION = re.compile(r"^\s*([0-9a-fA-F]{4}):[0-9a-fA-F]{8}\s+[0-9a-fA-F]+H\s+\S+\s+(CODE|DATA)\s*$")
MAP_SYMBOL = re.compile(r"^\s*(?:\d+:\s+)?([0-9a-fA-F]{4}):([0-9a-fA-F]{8})\s+(\S+)\s+([0-9a-fA-F]{16})\s")


def read_map(path):
    """Code symbols from an lld-link /MAP file (MSVC style)."""
    code_sections = set()
    entries = []
    with open(path, errors="replace") as f:
        for line in f:
            m = MAP_SECTION.match(line)
            if m:
                if m.group(2) == "CODE":
                    code_sections.add(int(m.group(1), 16))
                continue
            m = MAP_SYMBOL.match(line)
            if m and int(m.group(1), 16) in code_sections:
                entries.append((int(m.group(4), 16), 0, m.group(3)))
    if not entries:
        raise ValueError("no code symbols found in %s (is it an lld-link /MAP file?)" % path)
    return Symbols(entries)


def resolve_source(arg):
    """Maps a shorthand or path to (path, kind)."""
    if arg == "x64":
        path = os.path.join(ROOT, X64_MAP)
        if not os.path.exists(path):
            sys.exit("%s not found: build the x86_64 kernel first (the linker writes it)" % X64_MAP)
        return path, "map"
    if arg in SHORTHANDS:
        found = [p for pat in SHORTHANDS[arg] for prof in ("release", "debug")
                 for p in glob.glob(os.path.join(ROOT, pat.format(prof)))]
        if not found:
            sys.exit("no %s kernel build found under target/" % arg)
        return max(found, key=os.path.getmtime), "elf"
    if not os.path.exists(arg):
        sys.exit("%s: no such file" % arg)
    return arg, ("map" if arg.endswith(".map") else "elf")


# ----------------------------------------------------------------------------
# Demangling and line info (both optional)
# ----------------------------------------------------------------------------

def make_demangler():
    try:
        import rust_demangler

        def demangle(names):
            out = []
            for n in names:
                try:
                    out.append(rust_demangler.demangle(n))
                except Exception:
                    out.append(n)
            return out
        return demangle
    except ImportError:
        pass

    for tool in ("rustfilt", "llvm-cxxfilt", "c++filt"):
        exe = shutil.which(tool)
        if not exe:
            continue

        def demangle(names, exe=exe):
            try:
                done = subprocess.run([exe], input="\n".join(names) + "\n", capture_output=True,
                                      text=True, check=True)
                lines = done.stdout.split("\n")
                return lines[:len(names)] if len(lines) >= len(names) else names
            except Exception:
                return names
        return demangle

    return lambda names: names


def make_line_lookup(elf_path):
    """Returns f(address) -> 'file:line' or None, using an installed symbolizer."""
    for tool in ("llvm-symbolizer", "llvm-addr2line", "rust-addr2line", "addr2line"):
        exe = shutil.which(tool)
        if not exe:
            continue
        # Both families print "function\nfile:line" per address with these flags.
        base = [exe, "-e", elf_path, "-f", "-C"] if "addr2line" in tool else \
               [exe, "--obj=" + elf_path, "--functions=none", "--demangle", "--inlining=false"]

        def lookup(address, base=base, tool=tool):
            try:
                done = subprocess.run(base + ["0x%x" % address], capture_output=True, text=True, timeout=20)
                lines = [l for l in done.stdout.split("\n") if l.strip()]
                where = lines[-1] if lines else ""
                if where.startswith("?") or where.startswith("??"):
                    return None
                return os.path.relpath(where.split(" (")[0]) if os.path.isabs(where) else where
            except Exception:
                return None
        return lookup
    return None


# ----------------------------------------------------------------------------
# Log parsing
# ----------------------------------------------------------------------------

HEADER = re.compile(r"^=== DYNAMIX FAULT: (.*) ===")
ANCHOR = re.compile(r"^anchor:\s*(0x[0-9a-fA-F]+)")
FRAME = re.compile(r"^\s*#(\d+)\s+(0x[0-9a-fA-F]+)\s*(\S*)\s*$")


def split_reports(text):
    """Yields (header, lines) for every fault report in the log."""
    current = None
    for line in text.replace("\r", "").split("\n"):
        if HEADER.match(line):
            if current:
                yield current
            current = (line, [])
        elif current is not None:
            current[1].append(line)
    if current:
        yield current


def main():
    ap = argparse.ArgumentParser(description="Symbolize a Dynamix fault report.",
                                 usage="%(prog)s <aarch64|armv7|x64|file> [logfile]")
    ap.add_argument("source", help="aarch64, armv7, x64, or a path to an ELF image / .map file")
    ap.add_argument("log", nargs="?", help="log file (default: stdin)")
    args = ap.parse_args()

    path, kind = resolve_source(args.source)
    try:
        symbols = read_map(path) if kind == "map" else read_elf(path)
    except ValueError as e:
        sys.exit(str(e))
    print("symbols: %s" % os.path.relpath(path))

    if ANCHOR_SYMBOL not in symbols.by_name:
        print("warning: %s not in the symbol file; assuming load slide 0" % ANCHOR_SYMBOL)
    static_anchor = symbols.by_name.get(ANCHOR_SYMBOL)

    text = open(args.log, errors="replace").read() if args.log else sys.stdin.read()
    reports = list(split_reports(text))
    if not reports:
        sys.exit("no '=== DYNAMIX FAULT' report found in the log")

    demangle = make_demangler()
    line_lookup = make_line_lookup(path) if kind == "elf" else None

    for header, lines in reports:
        print()
        print(header)
        # cause / pc / addr, plus any continuation lines (a panic message has several)
        shown = False
        for l in lines:
            if l.startswith(("registers:", "backtrace")):
                break
            if l.startswith(("cause :", "pc    :", "addr  :")):
                shown = True
            if shown and l.strip():
                print(l)

        runtime_anchor = None
        for l in lines:
            m = ANCHOR.match(l)
            if m:
                runtime_anchor = int(m.group(1), 16)
        slide = 0
        if runtime_anchor is not None and static_anchor is not None:
            slide = runtime_anchor - static_anchor
            print("load slide: %#x" % slide)

        frames = []
        for l in lines:
            m = FRAME.match(l)
            if m:
                frames.append((int(m.group(1)), int(m.group(2), 16), m.group(3)))

        resolved = []
        for number, address, tag in frames:
            lookup_at = address - slide
            if tag != "pc":
                lookup_at -= 1  # a return address points after the call
            found = symbols.lookup(lookup_at)
            resolved.append((number, address, tag, lookup_at, found))

        names = demangle([r[4][0] for r in resolved if r[4]])
        names = iter(names)
        for number, address, tag, lookup_at, found in resolved:
            if not found:
                if tag != "?":  # stack-scan candidates that are not code are just noise
                    print("  #%-2d %#018x %-2s ??" % (number, address, tag))
                continue
            name = next(names)
            where = line_lookup(lookup_at) if line_lookup else None
            print("  #%-2d %#018x %-2s %s+%#x%s" % (number, address, tag, name, found[1],
                                                    "  (%s)" % where if where else ""))
        if any(t == "?" for _, _, t in frames):
            print("  (entries marked ? come from a stack scan: some may be stale)")


if __name__ == "__main__":
    main()
