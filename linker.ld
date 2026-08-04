ENTRY(_start)

SECTIONS
{
    . = 0x40200000;

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
}