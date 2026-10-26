/* SPDX-License-Identifier: GPL-3.0-or-later */
#define _GNU_SOURCE
#include <errno.h>
#include <signal.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/syscall.h>
#include <unistd.h>
#define CHECK(x) do { if (!(x)) { fprintf(stderr,"line %d: %s errno=%d\n",__LINE__,#x,errno); exit(1); } } while (0)
static unsigned char alt[65536] __attribute__((aligned(16)));
static volatile sig_atomic_t nested, seen, second, automatic;
static void handler(int sig, siginfo_t *info, void *context) {
    (void)context;
    CHECK(sig == SIGUSR1 && info->si_code == SI_TKILL && info->si_pid == getpid());
    unsigned char local; stack_t current;
    CHECK((uintptr_t)&local >= (uintptr_t)alt && (uintptr_t)&local < (uintptr_t)alt+sizeof alt);
    CHECK(sigaltstack(NULL, &current) == 0);
    if (automatic) CHECK(current.ss_flags & SS_DISABLE);
    else {
        CHECK(current.ss_flags & SS_ONSTACK);
        stack_t disabled = {.ss_flags=SS_DISABLE};
        CHECK(sigaltstack(&disabled, NULL) == -1 && errno == EPERM);
    }
    ++seen;
    if (!nested) { nested=1; CHECK(raise(SIGUSR1) == 0); }
}
static void clobber(int sig) {
    CHECK(sig == SIGUSR2);
    ++second;
    __asm__ volatile("movi v0.16b,#0\n msr fpcr,xzr\n msr fpsr,xzr" ::: "v0", "cc", "memory");
}
void probe(uint64_t *out, long pid);
__asm__(
    ".text\n .align 2\n .global probe\n .type probe,%function\n probe:\n"
    "mov x12,x0\n mov x0,x1\n mov x1,x0\n mov x2,#12\n mov x8,#131\n"
    "mrs x13,tpidr_el0\n mrs x14,fpcr\n mrs x15,fpsr\n"
    "mov x9,#0x400000\n msr fpcr,x9\n mov x9,#0x10\n msr fpsr,x9\n"
    "movi v0.16b,#0x5a\n mov x9,#-1\n adds x9,x9,#1\n svc #0\n"
    "cset x9,cs\n umov x10,v0.d[1]\n mrs x11,tpidr_el0\n eor x11,x11,x13\n"
    "stp x9,x10,[x12]\n str x11,[x12,#16]\n mrs x9,fpcr\n mrs x10,fpsr\n"
    "stp x9,x10,[x12,#24]\n msr fpcr,x14\n msr fpsr,x15\n ret\n .size probe,.-probe\n");
int main(int argc, char **argv) {
    automatic = argc > 1 && !strcmp(argv[1], "autodisarm");
    stack_t original, installed = {.ss_sp=alt, .ss_size=sizeof alt};
    if (automatic) installed.ss_flags=(int)0x80000000U;
    CHECK(sigaltstack(&installed, &original) == 0 && (original.ss_flags & SS_DISABLE));
    struct sigaction sa = {.sa_sigaction=handler, .sa_flags=SA_SIGINFO|SA_ONSTACK|SA_NODEFER};
    sigemptyset(&sa.sa_mask);
    CHECK(sigaction(SIGUSR1, &sa, NULL) == 0);
    CHECK(raise(SIGUSR1) == 0 && seen == 2);
    stack_t current;
    CHECK(sigaltstack(NULL, &current) == 0 && (unsigned)current.ss_flags == (automatic ? 0x80000000U : 0));
    sa.sa_handler=clobber; sa.sa_flags=0;
    CHECK(sigaction(SIGUSR2, &sa, NULL) == 0);
    sigset_t mask, old, pending;
    sigemptyset(&mask); sigaddset(&mask, SIGUSR2);
    CHECK(sigprocmask(SIG_BLOCK, &mask, &old) == 0);
    CHECK(raise(SIGUSR2) == 0 && raise(SIGUSR2) == 0 && second == 0);
    CHECK(sigpending(&pending) == 0 && sigismember(&pending, SIGUSR2));
    CHECK(sigprocmask(SIG_SETMASK, &old, NULL) == 0 && second == 1);
    uint64_t result[5]={0};
    probe(result, getpid());
    CHECK(second == 2 && result[0] == 1 && result[1] == UINT64_C(0x5a5a5a5a5a5a5a5a));
    CHECK(result[2] == 0 && result[3] == 0x400000 && result[4] == 0x10);
    sa.sa_flags=SA_RESETHAND;
    CHECK(sigaction(SIGUSR2, &sa, NULL) == 0 && raise(SIGUSR2) == 0 && second == 3);
    CHECK(sigaction(SIGUSR2, NULL, &sa) == 0 && sa.sa_handler == SIG_DFL);
    puts("task-signals: nested altstack, blocked coalescing, reset, NZCV/SIMD/FP/TLS restored");
    return 0;
}
