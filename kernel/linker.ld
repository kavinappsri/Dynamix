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