ENTRY(_start)

/* Nominal Lma is a bootkeeping anchor, for the ld file, its value doesnt change kernel logic at runtime */
NOMINAL_LMA = 0x00200000;

/* Virtual link adress for every symbol in this linker */
KERNEL_VMA = 0xFFFFFFC000000000;

/* Device table size */
_device_table_size = 64 * 4096;

SECTIONS
{
    . = KERNEL_VMA;
    _kernel_vma_start = .;

    .text.boot : AT(NOMINAL_LMA)
    {
        kernel_lma_start = LOADADDR(.text.boot);
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
        __start_serial_drivers = .;
        KEEP(*(.drivers.serial))
        __stop_serial_drivers = .;

        __start_framebuffer_drivers = .;
        KEEP(*(.drivers.framebuffer))
        __stop_framebuffer_drivers = .;
    }

    .data : AT(NOMINAL_LMA + (ADDR(.data) - _kernel_vma_start))
    {
        *(.data*)
    }

    .bss : AT(NOMINAL_LMA + (ADDR(.bss) - _kernel_vma_start))
    {
        . = ALIGN(16);
        _bss_start = .;
        *(.bss*)
        . = ALIGN(16);
        _bss_end = .;
    }

    .stack (NOLOAD) : AT(NOMINAL_LMA + (ADDR(.stack) - _kernel_vma_start))
    {
        . = ALIGN(16);
        _stack_bottom = .;
        . += 4096;
        . = ALIGN(16);
        _stack_top = .;
    }

    . = ALIGN(4096);

    .boottables (NOLOAD) : AT(NOMINAL_LMA + (ADDR(.boottables) - _kernel_vma_start))
    {
        _boottables_start = .;
        . += 4 * 4096;
        _boottables_end = .;
    }

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
