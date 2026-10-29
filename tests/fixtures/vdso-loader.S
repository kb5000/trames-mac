// SPDX-License-Identifier: GPL-3.0-or-later
// Link as a PIE with the real glibc PT_INTERP, without a libc dependency.
// The loader must register the kernel vDSO before reaching this entry point.
.text
.global _start
.type _start,%function
_start:
    mov x0,#1
    adr x1,message
    mov x2,#message_end-message
    mov x8,#64
    svc #0
    cmp x0,#message_end-message
    cset x0,ne
    mov x8,#94
    svc #0
.size _start,.-_start
message:
    .ascii "vDSO dynamic loader: ok\n"
message_end:
.section .note.GNU-stack,"",%progbits
