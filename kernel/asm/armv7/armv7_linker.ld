ENTRY(_start)

/* NOMINAL_LMA is a bootkeeping anchor, doesnt change kernel logic at runtime */
NOMINAL_LMA = 0x200000;

/* Virtual link address for every symbol in this linker */
KERNEL_VMA = 0x80000000;

/* Device table size -- L2 (small page) tables live here, 1KB each, 1KB-aligned */
_device_table_size = 64 * 1024;

SECTIONS {
    . = KERNEL_VMA;
    _kernel_vma_start = .;

    .text.boot : AT(NOMINAL_LMA)
    {
        _kernel_lma_start = LOADADDR(.text.boot);
        KEEP(*(.text.boot));
    }

    .text : AT(NOMINAL_LMA + (ADDR(.text) - _kernel_vma_start))
    {
        *(.text*)
    }

    .rodata : AT(NOMINAL_LMA + (ADDR(.rodata) - _kernel_vma_start))
    {
        *(.rodata*)
    }

    . = ALIGN(16);

    .drivers : AT(NOMINAL_LMA + (ADDR(.drivers) - _kernel_vma_start))
    {
        __start_drivers = .;
        __start_serial_drivers = .;
        KEEP(*(.drivers.serial))
        __stop_serial_drivers = .;

        __start_framebuffer_drivers = .;
        KEEP(*(.drivers.framebuffer))
        __stop_framebuffer_drivers = .;

        __start_clock_drivers = .;
        KEEP(*(.drivers.clocks))
        __stop_clock_drivers = .;
        __stop_drivers = .;
    }

    .data : AT(NOMINAL_LMA + (ADDR(.data) - _kernel_vma_start))
    {
        *(.data*)
    }

    .bss : AT(NOMINAL_LMA + (ADDR(.bss) - _kernel_vma_start))
    {
        . = ALIGN(4);
        _bss_start = .;
        *(.bss*)
        . = ALIGN(4);
        _bss_end = .;
    }

    .stack (NOLOAD) :
    {
        . = ALIGN(8);
        _stack_bottom = .;
        . += 4096; /* 4KB Stack Space */
        . = ALIGN(8);
        _stack_top = .;
    }

    . = ALIGN(16 * 1024);

    .boottables (NOLOAD) : AT(NOMINAL_LMA + (ADDR(.boottables) - _kernel_vma_start))
    {
        /*
         * 16 KiB TTBR0 L1, 16 KiB TTBR1 L1, then 32 x 1 KiB
         * coarse L2 tables for TTBR1's 32 MiB bootstrap mapping.
         */
        _boottables_start = .;
        . += 4 * (16 * 1024);
        _boottables_end = .;
    }

    . = ALIGN(1024);

    .devicetables (NOLOAD) : AT(NOMINAL_LMA + (ADDR(.devicetables) - _kernel_vma_start))
    {
        _devicetables_start = .;
        . += _device_table_size;
        _devicetables_end = .;
    }

    _kernel_vma_end = .;
    _kernel_lma_end = NOMINAL_LMA + (_kernel_vma_end - _kernel_vma_start);
    _image_size = _kernel_vma_end - _kernel_vma_start;
}
