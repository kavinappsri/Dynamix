ENTRY(_start)

SECTIONS
{
    . = 0x40200000;
    _kernel_start = .;

    .text :
    {
        *(.text._start)
        *(.text*)
    }

    .rodata :
    {
        *(.rodata*)
    }

    .drivers : ALIGN(16)
    {
        __start_serial_drivers = .;
        KEEP(*(.drivers.serial))
        __stop_serial_drivers = .;

        __start_framebuffer_drivers = .;
        KEEP(*(.drivers.framebuffer))
        __stop_framebuffer_drivers = .;
    }

    .data :
    {
        *(.data*)
    }

    .bss :
    {
        . = ALIGN(16);
        _bss_start = .;
        *(.bss*)
        . = ALIGN(16);
        _bss_end = .;
    }

    .stack (NOLOAD) :
    {
        . = ALIGN(16);
        _stack_bottom = .;
        . += 4096;
        . = ALIGN(16);
        _stack_top = .;
    }

    _kernel_end = .;
    _image_size = _kernel_end - _kernel_start;
}
