ENTRY(_start)

SECTIONS {
    . = 0x40400000;
    __image_start = .;

    .text : {
        KEEP(*(.text.boot))
        KEEP(*(.text.vectors))
        *(.text .text.*)
    }

    .rodata : ALIGN(4096) { *(.rodata .rodata.*) }
    .data   : ALIGN(4096) { *(.data .data.*) }

    .bss : ALIGN(4096) {
        __bss_start = .;
        *(.bss .bss.*)
        *(COMMON)
        . = ALIGN(16);
        __bss_end = .;
    }

    .stack (NOLOAD) : ALIGN(16) {
        . += 0x10000;
        __stack_top = .;
    }

    __image_end = .;
    __image_size = ABSOLUTE(__image_end - __image_start);

    /DISCARD/ : { *(.comment) *(.eh_frame*) }
}
