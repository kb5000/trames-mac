/* SPDX-License-Identifier: GPL-3.0-or-later */
#define _GNU_SOURCE
#include <pthread.h>
#include <signal.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>
#include <time.h>
#include <ucontext.h>

static volatile sig_atomic_t seen, bad;
static void illegal(int sig, siginfo_t *info, void *context) {
    ucontext_t *uc = context;
    if (sig != SIGILL || info->si_code != ILL_ILLOPC ||
        (uintptr_t)info->si_addr != uc->uc_mcontext.pc) bad = 1;
    seen++;
    uc->uc_mcontext.pc += 4;
}
static uint64_t counter(void) {
    uint64_t n;
    __asm__ volatile("isb; mrs %0,cntvct_el0" : "=r"(n) :: "memory");
    return n;
}
static void *check_counter(void *unused) {
    (void)unused;
    uint64_t hz, previous = counter();
    __asm__ volatile("mrs %0,cntfrq_el0" : "=r"(hz));
    if (!hz) return (void *)1;
    for (unsigned i = 0; i < 10000; i++) {
        uint64_t current = counter();
        if (current < previous) return (void *)2;
        previous = current;
    }
    struct timespec start, end;
    if (clock_gettime(CLOCK_MONOTONIC, &start)) return (void *)3;
    uint64_t first = counter();
    struct timespec delay = {0, 20000000};
    if (nanosleep(&delay, NULL)) return (void *)4;
    uint64_t last = counter();
    if (clock_gettime(CLOCK_MONOTONIC, &end)) return (void *)5;
    double elapsed = end.tv_sec - start.tv_sec + (end.tv_nsec - start.tv_nsec) / 1e9;
    double counted = (double)(last - first) / hz;
    if (counted < elapsed * .8 || counted > elapsed * 1.2) return (void *)6;
    return NULL;
}
int main(int argc, char **argv) {
    if (argc != 2) return 2;
    if (!strcmp(argv[1], "counter")) {
        pthread_t threads[4];
        for (unsigned i = 0; i < 4; i++)
            if (pthread_create(&threads[i], NULL, check_counter, NULL)) return 3;
        for (unsigned i = 0; i < 4; i++) {
            void *result;
            if (pthread_join(threads[i], &result) || result) return 4;
        }
        puts("EL0 virtual counter, frequency and threaded monotonic time: ok");
    } else if (!strcmp(argv[1], "privileged")) {
        struct sigaction action = {.sa_sigaction = illegal, .sa_flags = SA_SIGINFO};
        sigemptyset(&action.sa_mask);
        if (sigaction(SIGILL, &action, NULL)) return 5;
        __asm__ volatile("mrs xzr,cntv_ctl_el0\n\t"
                         "mrs xzr,sctlr_el1\n\t"
                         "msr cntkctl_el1,xzr" ::: "memory");
        if (seen != 3 || bad) return 6;
        puts("privileged timer and EL1 access deliver recoverable SIGILL: ok");
    } else return 7;
    return 0;
}
