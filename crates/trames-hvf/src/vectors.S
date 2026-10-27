// SPDX-License-Identifier: GPL-3.0-or-later
.text
.balign 2048
.global _trames_hvf_vectors_begin
.global _trames_hvf_vectors_end
_trames_hvf_vectors_begin:
// HVC #0 belongs to the platform SMCCC namespace. The framework may consume
// guest x0 values such as 0xc1000000 and return without an exception exit.
// A private immediate transports every Linux register value unchanged.
.rept 16
    hvc #0x5452
    eret
    .space 120
.endr
_trames_hvf_vectors_end:
.global _trames_hvf_tlbi_begin
.global _trames_hvf_tlbi_end
_trames_hvf_tlbi_begin:
    dsb ishst
    tlbi aside1is, x0
    dsb ish
    isb
    hvc #1
_trames_hvf_tlbi_end:

.global _trames_hvf_resume_begin
.global _trames_hvf_resume_end
_trames_hvf_resume_begin:
    clrex
    eret
_trames_hvf_resume_end:
