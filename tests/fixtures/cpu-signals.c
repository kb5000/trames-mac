/* SPDX-License-Identifier: GPL-3.0-or-later */
#define _GNU_SOURCE
#include <signal.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <ucontext.h>

static volatile sig_atomic_t seen[4];
static volatile sig_atomic_t bad;
static volatile sig_atomic_t expected_nzcv = -1;
static void handler(int sig, siginfo_t *info, void *context) {
    ucontext_t *uc = context;
    int slot = sig == SIGTRAP ? 0 : sig == SIGILL ? 1 : sig == SIGSEGV ? 2 : 3;
    seen[slot]++;
    if (expected_nzcv >= 0) {
        if ((uc->uc_mcontext.pstate >> 28 & 15) != (unsigned)expected_nzcv) bad = 1;
        /* A signal return replaces pending flags with the handler's edited
           architectural NZCV, even if the handler itself changed flags. */
        uc->uc_mcontext.pstate = (uc->uc_mcontext.pstate & ~(15ULL << 28)) | (10ULL << 28);
        expected_nzcv = -1;
    }
    if (sig == SIGSEGV) {
        if (info->si_code != SEGV_MAPERR && info->si_code != SEGV_ACCERR) bad = 1;
    } else if (sig == SIGILL) {
        /* Linux kernels use ILLOPC; QEMU user mode reports ILLOPN. */
        if (info->si_code != ILL_ILLOPC && info->si_code != ILL_ILLOPN) bad = 1;
    } else if (info->si_code != 1) bad = 1;
    if (sig == SIGTRAP || sig == SIGILL) {
        if ((uintptr_t)info->si_addr != uc->uc_mcontext.pc) bad = 1;
    }
    if (uc->uc_mcontext.pc == 0) uc->uc_mcontext.pc = uc->uc_mcontext.regs[30];
    else uc->uc_mcontext.pc += 4;
}
int main(void) {
    struct sigaction action;
    memset(&action, 0, sizeof(action));
    action.sa_sigaction = handler;
    action.sa_flags = SA_SIGINFO;
    sigemptyset(&action.sa_mask);
    int signals[] = {SIGTRAP, SIGILL, SIGSEGV, SIGBUS};
    for (unsigned i = 0; i < sizeof(signals)/sizeof(signals[0]); i++) if (sigaction(signals[i], &action, NULL)) return 2;
    __asm__ volatile("brk #123" ::: "memory");
    __asm__ volatile(".inst 0" ::: "memory");
    __asm__ volatile("hlt #123" ::: "memory");
    uintptr_t zero = 0;
    uint64_t value;
    __asm__ volatile("ldr %0,[%1]" : "=r"(value) : "r"(zero) : "memory");
    /* A misaligned ordered word access crossing a 16-byte atomicity boundary
       faults with or without the optional LSE2 relaxation in the oracle. */
    _Alignas(32) unsigned char data[32] = {0};
    const void *unaligned = data + 15;
    __asm__ volatile("ldar %w0,[%1]" : "=r"(value) : "r"(unaligned) : "memory");
    void *readonly = mmap(NULL, 4096, PROT_READ, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    if (readonly == MAP_FAILED) return 3;
    __asm__ volatile("str xzr,[%0]" :: "r"(readonly) : "memory");
    __asm__ volatile("blr %0" :: "r"(zero) : "x30", "memory");
    uint64_t observed;
    expected_nzcv = 6; /* UINT64_MAX + 1: Z and C, pending when BRK traps. */
    __asm__ volatile("adds xzr,%1,#1\n\tbrk #124\n\tmrs %0,nzcv"
                     : "=r"(observed) : "r"(UINT64_MAX) : "cc", "memory");
    if (observed != 10ULL << 28 || expected_nzcv != -1) bad = 1;
    expected_nzcv = 8; /* 0 - 1: N, pending when the load faults. */
    __asm__ volatile("subs xzr,%1,#1\n\tldr xzr,[%1]\n\tmrs %0,nzcv"
                     : "=r"(observed) : "r"(zero) : "cc", "memory");
    if (observed != 10ULL << 28 || expected_nzcv != -1) bad = 1;
    if (bad || seen[0] != 2 || seen[1] != 2 || seen[2] != 4 || seen[3] != 1) {
        printf("signal-failed %d %d %d %d %d\n", (int)bad, (int)seen[0], (int)seen[1], (int)seen[2], (int)seen[3]);
        return 4;
    }
    puts("cpu-signals-ok");
    return 0;
}
