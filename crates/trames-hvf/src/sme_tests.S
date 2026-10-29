// SPDX-License-Identifier: GPL-3.0-or-later
.arch armv9.2-a+sme2
.text
.balign 16
.global _trames_sme_test_begin, _trames_sme_test_end
_trames_sme_test_begin:
    smstart
    rdsvl x2, #1
    ptrue p0.s
    fdup z0.s, #1.0
    fdup z1.s, #2.0
    zero {za}
    ldr zt0, [x0]
    fmopa za0.s, p0/m, p0/m, z0.s, z1.s
    mov x1, #0x713
    msr tpidr2_el0, x1
    svc #0
    smstart sm
    ptrue p0.s
    fdup z0.s, #1.0
    fdup z1.s, #2.0
    fmopa za0.s, p0/m, p0/m, z0.s, z1.s
    add x1, x0, #64
    str zt0, [x1]
    mrs x3, tpidr2_el0
    svc #0
_trames_sme_test_end:
.balign 16
.global _trames_sme_yield_begin, _trames_sme_yield_loop, _trames_sme_yield_finish, _trames_sme_yield_end
_trames_sme_yield_begin:
    smstart
    ptrue p0.s
    fdup z0.s, #1.0
    fdup z1.s, #2.0
    zero {za}
    ldr zt0, [x0]
    fmopa za0.s, p0/m, p0/m, z0.s, z1.s
    mov x1, #0x713
    msr tpidr2_el0, x1
_trames_sme_yield_loop:
    b _trames_sme_yield_loop
_trames_sme_yield_finish:
    fmopa za0.s, p0/m, p0/m, z0.s, z1.s
    svc #0
_trames_sme_yield_end:
