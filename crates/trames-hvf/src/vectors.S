// SPDX-License-Identifier: GPL-3.0-or-later
// Per-vCPU EL1-only context at SP_EL1. No guest stack or host pointer.
.arch armv9.2-a+sme2
.text
.balign 2048
.global _trames_hvf_vectors_begin
.global _trames_hvf_vectors_end
_trames_hvf_vectors_begin:
.rept 16
    b _trames_hvf_save_begin
    .space 124
.endr
.global _trames_hvf_tlbi_begin
.global _trames_hvf_tlbi_end
_trames_hvf_tlbi_begin:
    ldr x0, [sp, #840]
    cbz x0, _trames_hvf_resume_begin
    dsb ishst
    cmp x0, #3
    b.eq 3f
    ldr x1, [sp, #848]
    cmp x0, #1
    b.ne 1f
    tlbi vae1is, x1
    b 2f
1:  tlbi aside1is, x1
2:  dsb ish
3:  isb
    str xzr, [sp, #840]
    b _trames_hvf_resume_begin
_trames_hvf_tlbi_end:
.space 0x900 - (. - _trames_hvf_vectors_begin)
.global _trames_hvf_resume_begin
.global _trames_hvf_resume_end
_trames_hvf_resume_begin:
    clrex
    bl _trames_hvf_sme_prepare
    ldr x0, [sp, #248]
    msr sp_el0, x0
    ldr x0, [sp, #256]
    msr elr_el1, x0
    ldr x0, [sp, #272]
    msr tpidr_el0, x0
    ldr w0, [sp, #264]
    and x0, x0, #0xf0000000
    orr x0, x0, #0x3c0
    msr spsr_el1, x0
    ldr w0, [sp, #288]
    msr fpcr, x0
    ldr w0, [sp, #292]
    msr fpsr, x0
    ldp q0, q1, [sp, #304]
    ldp q2, q3, [sp, #336]
    ldp q4, q5, [sp, #368]
    ldp q6, q7, [sp, #400]
    ldp q8, q9, [sp, #432]
    ldp q10, q11, [sp, #464]
    ldp q12, q13, [sp, #496]
    ldp q14, q15, [sp, #528]
    ldp q16, q17, [sp, #560]
    ldp q18, q19, [sp, #592]
    ldp q20, q21, [sp, #624]
    ldp q22, q23, [sp, #656]
    ldp q24, q25, [sp, #688]
    ldp q26, q27, [sp, #720]
    ldp q28, q29, [sp, #752]
    ldp q30, q31, [sp, #784]
    bl _trames_hvf_sme_restore
    ldp x2, x3, [sp, #16]
    ldp x4, x5, [sp, #32]
    ldp x6, x7, [sp, #48]
    ldp x8, x9, [sp, #64]
    ldp x10, x11, [sp, #80]
    ldp x12, x13, [sp, #96]
    ldp x14, x15, [sp, #112]
    ldp x16, x17, [sp, #128]
    ldp x18, x19, [sp, #144]
    ldp x20, x21, [sp, #160]
    ldp x22, x23, [sp, #176]
    ldp x24, x25, [sp, #192]
    ldp x26, x27, [sp, #208]
    ldp x28, x29, [sp, #224]
    ldr x30, [sp, #240]
    ldp x0, x1, [sp]
    eret
_trames_hvf_resume_end:
// The helpers are part of the interrupted-restore interval. GPRs have not
// been restored yet; x30 is scratch until the final bank load.
_trames_hvf_sme_prepare:
    ldr x0, [sp, #864]
    cbz x0, 9f
    isb
    smstop
    ldr x1, [sp, #880]
    lsr x1, x1, #4
    sub x1, x1, #1
    tbz x0, #1, 1f
    orr x1, x1, #0x40000000 // SMCR_EL1.EZT0
1:  msr smcr_el1, x1
    ldr x1, [sp, #280]
    msr tpidr2_el0, x1
    isb
9:  ret
_trames_hvf_sme_restore:
    ldr x0, [sp, #864]
    cbz x0, 9f
    ldr x2, [sp, #888]
    tbz x2, #1, 2f
    smstart za
    add x1, sp, #3072
    ldr x3, [sp, #880]
    mov w12, #0
1:  ldr za[w12, 0], [x1]
    add x1, x1, x3
    add w12, w12, #1
    cmp x12, x3
    b.lo 1b
    tbz x0, #1, 2f
    add x1, sp, #1, lsl #12
    add x1, x1, #3072
    ldr zt0, [x1]
2:  tbz x2, #0, 9f
    smstart sm
    add x1, sp, #896
    ldr z0, [x1, #0, mul vl]
    ldr z1, [x1, #1, mul vl]
    ldr z2, [x1, #2, mul vl]
    ldr z3, [x1, #3, mul vl]
    ldr z4, [x1, #4, mul vl]
    ldr z5, [x1, #5, mul vl]
    ldr z6, [x1, #6, mul vl]
    ldr z7, [x1, #7, mul vl]
    ldr z8, [x1, #8, mul vl]
    ldr z9, [x1, #9, mul vl]
    ldr z10, [x1, #10, mul vl]
    ldr z11, [x1, #11, mul vl]
    ldr z12, [x1, #12, mul vl]
    ldr z13, [x1, #13, mul vl]
    ldr z14, [x1, #14, mul vl]
    ldr z15, [x1, #15, mul vl]
    ldr z16, [x1, #16, mul vl]
    ldr z17, [x1, #17, mul vl]
    ldr z18, [x1, #18, mul vl]
    ldr z19, [x1, #19, mul vl]
    ldr z20, [x1, #20, mul vl]
    ldr z21, [x1, #21, mul vl]
    ldr z22, [x1, #22, mul vl]
    ldr z23, [x1, #23, mul vl]
    ldr z24, [x1, #24, mul vl]
    ldr z25, [x1, #25, mul vl]
    ldr z26, [x1, #26, mul vl]
    ldr z27, [x1, #27, mul vl]
    ldr z28, [x1, #28, mul vl]
    ldr z29, [x1, #29, mul vl]
    ldr z30, [x1, #30, mul vl]
    ldr z31, [x1, #31, mul vl]
    add x1, sp, #2944
    ldr p0, [x1, #0, mul vl]
    ldr p1, [x1, #1, mul vl]
    ldr p2, [x1, #2, mul vl]
    ldr p3, [x1, #3, mul vl]
    ldr p4, [x1, #4, mul vl]
    ldr p5, [x1, #5, mul vl]
    ldr p6, [x1, #6, mul vl]
    ldr p7, [x1, #7, mul vl]
    ldr p8, [x1, #8, mul vl]
    ldr p9, [x1, #9, mul vl]
    ldr p10, [x1, #10, mul vl]
    ldr p11, [x1, #11, mul vl]
    ldr p12, [x1, #12, mul vl]
    ldr p13, [x1, #13, mul vl]
    ldr p14, [x1, #14, mul vl]
    ldr p15, [x1, #15, mul vl]
9:  ret
.space 0x1000 - (. - _trames_hvf_vectors_begin)
.global _trames_hvf_save_begin
.global _trames_hvf_save_end
_trames_hvf_save_begin:
    stp x0, x1, [sp, #0]
    stp x2, x3, [sp, #16]
    stp x4, x5, [sp, #32]
    stp x6, x7, [sp, #48]
    stp x8, x9, [sp, #64]
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
    str x30, [sp, #240]
    bl _trames_hvf_sme_save
    stp q0, q1, [sp, #304]
    stp q2, q3, [sp, #336]
    stp q4, q5, [sp, #368]
    stp q6, q7, [sp, #400]
    stp q8, q9, [sp, #432]
    stp q10, q11, [sp, #464]
    stp q12, q13, [sp, #496]
    stp q14, q15, [sp, #528]
    stp q16, q17, [sp, #560]
    stp q18, q19, [sp, #592]
    stp q20, q21, [sp, #624]
    stp q22, q23, [sp, #656]
    stp q24, q25, [sp, #688]
    stp q26, q27, [sp, #720]
    stp q28, q29, [sp, #752]
    stp q30, q31, [sp, #784]
    mrs x0, sp_el0
    str x0, [sp, #248]
    mrs x0, elr_el1
    str x0, [sp, #256]
    mrs x0, spsr_el1
    str w0, [sp, #264]
    mrs x0, tpidr_el0
    str x0, [sp, #272]
    mrs x0, fpcr
    str w0, [sp, #288]
    mrs x0, fpsr
    str w0, [sp, #292]
    mrs x0, esr_el1
    str x0, [sp, #816]
    mrs x0, far_el1
    str x0, [sp, #824]
    mov x0, #1
    str x0, [sp, #832]
// HVC #0 is reserved for platform SMCCC. Preserve the private ABI.
    hvc #0x5452
    b _trames_hvf_resume_begin
// Save streaming vectors before SMSTOP, which zeros their architectural
// contents. ZA can be transferred with SM=0; no FA64-dependent FFR is used.
_trames_hvf_sme_save:
    ldr x0, [sp, #864]
    cbz x0, 9f
    mrs x2, svcr
    str x2, [sp, #888]
    mrs x1, tpidr2_el0
    str x1, [sp, #280]
    tbz x2, #0, 2f
    add x1, sp, #896
    str z0, [x1, #0, mul vl]
    str z1, [x1, #1, mul vl]
    str z2, [x1, #2, mul vl]
    str z3, [x1, #3, mul vl]
    str z4, [x1, #4, mul vl]
    str z5, [x1, #5, mul vl]
    str z6, [x1, #6, mul vl]
    str z7, [x1, #7, mul vl]
    str z8, [x1, #8, mul vl]
    str z9, [x1, #9, mul vl]
    str z10, [x1, #10, mul vl]
    str z11, [x1, #11, mul vl]
    str z12, [x1, #12, mul vl]
    str z13, [x1, #13, mul vl]
    str z14, [x1, #14, mul vl]
    str z15, [x1, #15, mul vl]
    str z16, [x1, #16, mul vl]
    str z17, [x1, #17, mul vl]
    str z18, [x1, #18, mul vl]
    str z19, [x1, #19, mul vl]
    str z20, [x1, #20, mul vl]
    str z21, [x1, #21, mul vl]
    str z22, [x1, #22, mul vl]
    str z23, [x1, #23, mul vl]
    str z24, [x1, #24, mul vl]
    str z25, [x1, #25, mul vl]
    str z26, [x1, #26, mul vl]
    str z27, [x1, #27, mul vl]
    str z28, [x1, #28, mul vl]
    str z29, [x1, #29, mul vl]
    str z30, [x1, #30, mul vl]
    str z31, [x1, #31, mul vl]
    add x1, sp, #2944
    str p0, [x1, #0, mul vl]
    str p1, [x1, #1, mul vl]
    str p2, [x1, #2, mul vl]
    str p3, [x1, #3, mul vl]
    str p4, [x1, #4, mul vl]
    str p5, [x1, #5, mul vl]
    str p6, [x1, #6, mul vl]
    str p7, [x1, #7, mul vl]
    str p8, [x1, #8, mul vl]
    str p9, [x1, #9, mul vl]
    str p10, [x1, #10, mul vl]
    str p11, [x1, #11, mul vl]
    str p12, [x1, #12, mul vl]
    str p13, [x1, #13, mul vl]
    str p14, [x1, #14, mul vl]
    str p15, [x1, #15, mul vl]
2:  tbz x2, #1, 4f
    add x1, sp, #3072
    ldr x3, [sp, #880]
    mov w12, #0
3:  str za[w12, 0], [x1]
    add x1, x1, x3
    add w12, w12, #1
    cmp x12, x3
    b.lo 3b
    tbz x0, #1, 4f
    add x1, sp, #1, lsl #12
    add x1, x1, #3072
    str zt0, [x1]
4:  tbz x2, #0, 9f
    smstop sm
    // Rebuild the base V bank from low Z bits without a framework call.
    add x1, sp, #896
    ldr x3, [sp, #880]
    ldr q0, [x1]
    add x1, x1, x3
    ldr q1, [x1]
    add x1, x1, x3
    ldr q2, [x1]
    add x1, x1, x3
    ldr q3, [x1]
    add x1, x1, x3
    ldr q4, [x1]
    add x1, x1, x3
    ldr q5, [x1]
    add x1, x1, x3
    ldr q6, [x1]
    add x1, x1, x3
    ldr q7, [x1]
    add x1, x1, x3
    ldr q8, [x1]
    add x1, x1, x3
    ldr q9, [x1]
    add x1, x1, x3
    ldr q10, [x1]
    add x1, x1, x3
    ldr q11, [x1]
    add x1, x1, x3
    ldr q12, [x1]
    add x1, x1, x3
    ldr q13, [x1]
    add x1, x1, x3
    ldr q14, [x1]
    add x1, x1, x3
    ldr q15, [x1]
    add x1, x1, x3
    ldr q16, [x1]
    add x1, x1, x3
    ldr q17, [x1]
    add x1, x1, x3
    ldr q18, [x1]
    add x1, x1, x3
    ldr q19, [x1]
    add x1, x1, x3
    ldr q20, [x1]
    add x1, x1, x3
    ldr q21, [x1]
    add x1, x1, x3
    ldr q22, [x1]
    add x1, x1, x3
    ldr q23, [x1]
    add x1, x1, x3
    ldr q24, [x1]
    add x1, x1, x3
    ldr q25, [x1]
    add x1, x1, x3
    ldr q26, [x1]
    add x1, x1, x3
    ldr q27, [x1]
    add x1, x1, x3
    ldr q28, [x1]
    add x1, x1, x3
    ldr q29, [x1]
    add x1, x1, x3
    ldr q30, [x1]
    add x1, x1, x3
    ldr q31, [x1]
    add x1, x1, x3
9:  ret
_trames_hvf_save_end:
_trames_hvf_vectors_end:
