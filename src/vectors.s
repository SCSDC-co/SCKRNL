.equ FRAME_SIZE, 288

.section .text.vectors, "ax"

.macro entry idx, target
    .balign 0x80
    sub     sp, sp, #FRAME_SIZE
    stp     x0, x1, [sp]
    mov     x0, #\idx
    b       \target
.endm

.macro vector_table name, target
    .balign 0x800
    .global \name
\name:
    entry 0,  \target       // current EL, SP0: sync
    entry 1,  \target       //                  irq
    entry 2,  \target       //                  fiq
    entry 3,  \target       //                  serror
    entry 4,  \target       // current EL, SPx: sync
    entry 5,  \target
    entry 6,  \target
    entry 7,  \target
    entry 8,  \target       // lower EL, AArch64: sync
    entry 9,  \target
    entry 10, \target
    entry 11, \target
    entry 12, \target       // lower EL, AArch32: sync
    entry 13, \target
    entry 14, \target
    entry 15, \target
.endm

// Common path. On entry: x0 = index, original x0/x1 are already in the frame.
.macro exc_common el, label
\label:
    stp x2,  x3,  [sp, #16]
    stp x4,  x5,  [sp, #32]
    stp x6,  x7,  [sp, #48]
    stp x8,  x9,  [sp, #64]
    stp x10, x11, [sp, #80]
    stp x12, x13, [sp, #96]
    stp x14, x15, [sp, #112]
    stp x16, x17, [sp, #128]
    stp x18, x19, [sp, #144]
    stp x20, x21, [sp, #160]
    stp x22, x23, [sp, #176]
    stp x24, x25, [sp, #192]
    stp x26, x27, [sp, #208]
    stp x28, x29, [sp, #224]
    str     x30, [sp, #240]

    mrs     x2, elr_el\el
    mrs     x3, spsr_el\el
    mrs     x4, esr_el\el
    mrs     x5, far_el\el
    str x2, [sp, #248]
    str x3, [sp, #256]
    str x4, [sp, #264]
    str x5, [sp, #272]

    mov     x1, x0
    mov     x0, sp
    bl      {handler}
    b       .
.endm

vector_table el2_vectors, exc_common_el2
exc_common 2, exc_common_el2

vector_table el1_vectors, exc_common_el1
exc_common 1, exc_common_el1
