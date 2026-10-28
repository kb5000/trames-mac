// SPDX-License-Identifier: GPL-3.0-or-later
// Per-vCPU EL1-only context at SP_EL1. No guest stack or host pointer.
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
.space 0xa00 - (. - _trames_hvf_vectors_begin)
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
_trames_hvf_save_end:
_trames_hvf_vectors_end:
