// SPDX-License-Identifier: GPL-3.0-or-later
// Fixed-work local mapping churn with two independent Linux processes, followed
// by one-byte writes to a 4 MiB mapped file. No throughput threshold is asserted.
#define _GNU_SOURCE
#include <errno.h>
#include <stdatomic.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <sys/mman.h>
#include <sys/wait.h>
#include <time.h>
#include <unistd.h>

#define PAGE 4096
struct shared {
    _Atomic unsigned ready;
    _Atomic unsigned stop;
    _Alignas(64) _Atomic uint64_t count[2];
};
static void require(int condition) {
    if (!condition) { perror("mapping-scope"); exit(1); }
}
static uint64_t now(void) {
    struct timespec t;
    require(clock_gettime(CLOCK_MONOTONIC, &t) == 0);
    return (uint64_t)t.tv_sec * 1000000000 + t.tv_nsec;
}
int main(void) {
    int library = memfd_create("shared-library", 0);
    int control = memfd_create("shared-control", 0);
    require(library >= 0 && control >= 0);
    require(ftruncate(library, 1024 * PAGE) == 0 && ftruncate(control, PAGE) == 0);
    unsigned char *image = mmap(0, 1024 * PAGE, PROT_READ, MAP_PRIVATE, library, 0);
    struct shared *state = mmap(0, PAGE, PROT_READ | PROT_WRITE, MAP_SHARED, control, 0);
    require(image != MAP_FAILED && state != MAP_FAILED);
    volatile unsigned checksum = 0;
    for (int i = 0; i < 1024; ++i) checksum += image[i * PAGE];
    pid_t children[2];
    for (int i = 0; i < 2; ++i) {
        children[i] = fork();
        require(children[i] >= 0);
        if (children[i] == 0) {
            atomic_fetch_add_explicit(&state->ready, 1, memory_order_release);
            uint64_t count = 0;
            while (!atomic_load_explicit(&state->stop, memory_order_relaxed)) {
                count += 1;
                atomic_store_explicit(&state->count[i], count, memory_order_relaxed);
            }
            _exit(0);
        }
    }
    struct timespec delay = {0, 1000000};
    while (atomic_load_explicit(&state->ready, memory_order_acquire) != 2)
        nanosleep(&delay, 0);
    uint64_t before[2];
    for (int i = 0; i < 2; ++i) before[i] = atomic_load_explicit(&state->count[i], memory_order_relaxed);
    uint64_t began = now();
    for (int i = 0; i < 400; ++i) {
        void *p = mmap(0, PAGE, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
        require(p != MAP_FAILED);
        require(mprotect(p, PAGE, PROT_READ) == 0);
        require(munmap(p, PAGE) == 0);
    }
    uint64_t duration = now() - began;
    uint64_t progress[2];
    for (int i = 0; i < 2; ++i) progress[i] = atomic_load_explicit(&state->count[i], memory_order_relaxed) - before[i];
    atomic_store_explicit(&state->stop, 1, memory_order_relaxed);
    for (int i = 0; i < 2; ++i) {
        int status;
        require(waitpid(children[i], &status, 0) == children[i]);
        require(WIFEXITED(status) && WEXITSTATUS(status) == 0);
    }
    printf("SCOPE ns=%llu peer0=%llu peer1=%llu checksum=%u\n",
        (unsigned long long)duration, (unsigned long long)progress[0],
        (unsigned long long)progress[1], checksum);
    void *p = mmap(0, PAGE, PROT_READ, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    require(p != MAP_FAILED);
    began = now();
    for (int i = 0; i < 128; ++i) {
        unsigned char value = (unsigned char)i;
        require(pwrite(library, &value, 1, 3 * PAGE) == 1);
        require(mprotect(p, PAGE, (i & 1) ? PROT_READ : PROT_READ | PROT_WRITE) == 0);
        require(image[3 * PAGE] == value);
    }
    printf("RANGE ns=%llu value=%u\n", (unsigned long long)(now() - began), image[3 * PAGE]);
    require(munmap(p, PAGE) == 0);
    return 0;
}
