ENTRY(_start)

SECTIONS {
    . = 40200000;
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

    .drivers :
    {
        __start_serial_drivers = .;
        KEEP(*(.drivers.serial))
        __stop_serial_drivers = .;

        __start_framebuffer_drivers = .;
        KEEP(*(.drivers.framebuffer))
        __stop_serial_drivers = .;
    }

    data :
    {
        *(.data*)
    }

    .bss :
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

    _kernel_end  = .;
    _image_size = _kernel_start - _kernel_end;
}