// SPDX-License-Identifier: GPL-3.0-or-later
.arch armv9.2-a+sme2
.text
.balign 4
.global sme_signal_probe
.type sme_signal_probe,%function
sme_signal_probe:
    sub sp,sp,#128
    stp q8,q9,[sp]
    stp q10,q11,[sp,#32]
    stp q12,q13,[sp,#64]
    stp q14,q15,[sp,#96]
    mov x9,x0
    smstart
    ptrue p0.s
    fdup z0.s,#1.0
    fdup z1.s,#2.0
    zero {za}
    ldr zt0,[x9]
    fmopa za0.s,p0/m,p0/m,z0.s,z1.s
    mov x10,#0x713
    msr tpidr2_el0,x10
1:  ldr w10,[x1]
    cbz w10,1b
    fmopa za0.s,p0/m,p0/m,z0.s,z1.s
    add x10,x9,#128
    mov w12,#0
2:  str za[w12,0],[x10]
    add x10,x10,#64
    add w12,w12,#1
    cmp w12,#64
    b.lo 2b
    add x10,x9,#64
    str zt0,[x10]
    mrs x10,svcr
    str x10,[x9,#4224]
    mrs x10,tpidr2_el0
    str x10,[x9,#4232]
    smstop
    msr tpidr2_el0,xzr
    ldp q8,q9,[sp]
    ldp q10,q11,[sp,#32]
    ldp q12,q13,[sp,#64]
    ldp q14,q15,[sp,#96]
    add sp,sp,#128
    ret
.size sme_signal_probe,.-sme_signal_probe
.global sme_clobber
.type sme_clobber,%function
sme_clobber:
    sub sp,sp,#128
    stp q8,q9,[sp]
    stp q10,q11,[sp,#32]
    stp q12,q13,[sp,#64]
    stp q14,q15,[sp,#96]
    smstart
    ptrue p0.s
    fdup z0.s,#3.0
    zero {za}
    zero {zt0}
    fmopa za0.s,p0/m,p0/m,z0.s,z0.s
    smstop
    ldp q8,q9,[sp]
    ldp q10,q11,[sp,#32]
    ldp q12,q13,[sp,#64]
    ldp q14,q15,[sp,#96]
    add sp,sp,#128
    ret
.size sme_clobber,.-sme_clobber
.section .note.GNU-stack,"",%progbits
