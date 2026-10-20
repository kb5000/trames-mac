// SPDX-License-Identifier: GPL-3.0-or-later
// An asynchronous signal may interrupt AArch64 leaf assembly while SP is only
// 8-byte aligned. Linux aligns the handler frame and restores interrupted SP.
#define _GNU_SOURCE
#include <signal.h>
#include <stdint.h>
#include <stdio.h>
#include <unistd.h>
#include <sys/syscall.h>
static volatile sig_atomic_t seen;
static void handler(int sig) { if(sig==SIGUSR1) ++seen; }
int main(void) {
    struct sigaction action={.sa_handler=handler};
    sigemptyset(&action.sa_mask);
    if(sigaction(SIGUSR1,&action,0))return 1;
    register long x0 __asm__("x0")=getpid();
    register long x1 __asm__("x1")=syscall(SYS_gettid);
    register long x2 __asm__("x2")=SIGUSR1;
    register long x8 __asm__("x8")=SYS_tgkill;
    uintptr_t before,after;
    __asm__ volatile("mov %0, sp" : "=r"(before));
    __asm__ volatile("sub sp, sp, #8\n svc #0\n add sp, sp, #8"
        : "+r"(x0) : "r"(x1),"r"(x2),"r"(x8) : "memory","cc");
    __asm__ volatile("mov %0, sp" : "=r"(after));
    if(x0 || seen!=1 || before!=after)return 2;
    puts("signal-leaf-stack: aligned handler, interrupted SP restored");
    return 0;
}
