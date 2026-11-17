.section .text.boot
.global _start
_start:
    b       _real_start
    .long   0
    .quad   0
    .quad   __image_size
    .quad   0xa
    .quad   0, 0, 0
    .ascii  "ARM\x64"
    .long   0

_real_start:
    mov     x19, x0
    mrs     x20, CurrentEL
    lsr     x20, x20, #2

    mrs     x0, mpidr_el1
    and     x0, x0, #0xff
    cbnz    x0, .Lpark

    cmp     x20, #2
    b.ne    .Lel1

    // --- Entered at EL2 ---
    ldr     x0, =__stack_top
    mov     sp, x0
    adrp    x0, el2_vectors
    add     x0, x0, :lo12:el2_vectors
    msr     vbar_el2, x0
    isb

    mov     x0, #(1 << 31)
    msr     hcr_el2, x0
    mov     x0, #3
    msr     cnthctl_el2, x0
    msr     cntvoff_el2, xzr
    mov     x0, #0x0800
    movk    x0, #0x30d0, lsl #16
    msr     sctlr_el1, x0
    mov     x0, #0x3c5
    msr     spsr_el2, x0
    adr     x0, .Lel1
    msr     elr_el2, x0
    eret

.Lel1:
    ldr     x0, =__stack_top
    mov     sp, x0
    adrp    x0, el1_vectors
    add     x0, x0, :lo12:el1_vectors
    msr     vbar_el1, x0
    isb

    ldr     x0, =__bss_start
    ldr     x1, =__bss_end
.Lbss:
    cmp     x0, x1
    b.hs    .Lrun
    str     xzr, [x0], #8
    b       .Lbss

.Lrun:
    mov     x0, x19
    mov     x1, x20
    bl      kmain
.Lpark:
    wfe
    b       .Lpark
