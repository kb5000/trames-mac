// SPDX-License-Identifier: GPL-3.0-or-later
.text
.balign 2048
.global _trames_hvf_vectors_begin
.global _trames_hvf_vectors_end
_trames_hvf_vectors_begin:
.rept 16
    hvc #0
    eret
    .space 120
.endr
_trames_hvf_vectors_end:
.global _trames_hvf_tlbi_begin
.global _trames_hvf_tlbi_end
_trames_hvf_tlbi_begin:
    dsb ishst
    tlbi vmalle1
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
