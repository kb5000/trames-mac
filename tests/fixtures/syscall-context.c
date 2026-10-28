// SPDX-License-Identifier: GPL-3.0-or-later
// Fixed-work short-syscall throughput; excludes process and library startup.
#define _GNU_SOURCE
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <sys/syscall.h>
#include <time.h>
#include <unistd.h>
static uint64_t now(void) {
    struct timespec t;
    if (clock_gettime(CLOCK_MONOTONIC, &t)) abort();
    return (uint64_t)t.tv_sec * 1000000000 + t.tv_nsec;
}
int main(void) {
    const unsigned rounds = 300000;
    long expected = syscall(SYS_getpid);
    for (unsigned i = 0; i < 4096; ++i)
        if (syscall(SYS_getpid) != expected) abort();
    uint64_t sum = 0, began = now();
    for (unsigned i = 0; i < rounds; ++i)
        sum += (uint64_t)syscall(SYS_getpid);
    uint64_t elapsed = now() - began;
    if (sum != (uint64_t)expected * rounds) abort();
    printf("SYSCALL rounds=%u ns=%llu checksum=%llu\n", rounds,
        (unsigned long long)elapsed, (unsigned long long)sum);
    return 0;
}
