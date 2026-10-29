// SPDX-License-Identifier: GPL-3.0-or-later
#define _GNU_SOURCE
#include <assert.h>
#include <pthread.h>
#include <sched.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <time.h>
#include <unistd.h>
static unsigned threads, rounds;
static int churn;
static volatile uint64_t *shared;
static size_t pages;
static pthread_barrier_t start;
static uint64_t sums[32];
static uint64_t now(void) {
    struct timespec t; assert(!clock_gettime(CLOCK_MONOTONIC, &t));
    return (uint64_t)t.tv_sec * 1000000000 + t.tv_nsec;
}
static void *work(void *argument) {
    uintptr_t id = (uintptr_t)argument;
    int r = pthread_barrier_wait(&start); assert(!r || r == PTHREAD_BARRIER_SERIAL_THREAD);
    uint64_t sum = 0;
    for (unsigned round = 0; round < rounds; ++round) {
        if (churn) {
            volatile uint64_t *p = mmap(NULL, pages * 4096, PROT_READ | PROT_WRITE,
                                       MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
            assert(p != MAP_FAILED);
            for (size_t i = 0; i < pages; ++i) p[i * 512] = id + round + i;
            for (size_t i = 0; i < pages; ++i) sum += p[i * 512];
            assert(!munmap((void *)p, pages * 4096));
        } else {
            for (size_t i = 0; i < pages; ++i) sum += shared[i * 512];
            assert(!sched_yield());
        }
    }
    sums[id] = sum;
    return NULL;
}
int main(int argc, char **argv) {
    assert(argc == 5); churn = !strcmp(argv[1], "churn");
    assert(churn || !strcmp(argv[1], "reuse"));
    threads = atoi(argv[2]); rounds = atoi(argv[3]); pages = strtoul(argv[4], NULL, 10);
    assert(threads >= 1 && threads <= 32 && rounds && pages);
    uint64_t began = now();
    if (!churn) {
        shared = mmap(NULL, pages * 4096, PROT_READ | PROT_WRITE,
                      MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
        assert(shared != MAP_FAILED);
        for (size_t i = 0; i < pages; ++i) shared[i * 512] = i + 1;
        // Read-only VMA selects the sparse 4K translation path, rather than
        // benchmarking an already-supported uniform RW native 2MiB block.
        assert(!mprotect((void *)shared, pages * 4096, PROT_READ));
        uint64_t warm = 0;
        for (size_t i = 0; i < pages; ++i) warm += shared[i * 512];
        assert(warm == (uint64_t)pages * (pages + 1) / 2);
    }
    pthread_t workers[32]; assert(!pthread_barrier_init(&start, NULL, threads + 1));
    for (uintptr_t i = 0; i < threads; ++i) assert(!pthread_create(&workers[i], NULL, work, (void *)i));
    uint64_t compute = now();
    int r = pthread_barrier_wait(&start); assert(!r || r == PTHREAD_BARRIER_SERIAL_THREAD);
    uint64_t sum = 0;
    for (unsigned i = 0; i < threads; ++i) {
        assert(!pthread_join(workers[i], NULL));
        uint64_t expected = churn ?
            (uint64_t)rounds * (pages * i + pages * (pages - 1) / 2) +
                pages * (uint64_t)rounds * (rounds - 1) / 2 :
            (uint64_t)rounds * pages * (pages + 1) / 2;
        assert(sums[i] == expected); sum += sums[i];
    }
    uint64_t ended = now();
    if (!churn) assert(!munmap((void *)shared, pages * 4096));
    printf("{\"mode\":\"%s\",\"threads\":%u,\"rounds\":%u,\"pages\":%zu,\"work_ns\":%llu,\"total_ns\":%llu,\"checksum\":%llu}\n",
           churn ? "churn" : "reuse", threads, rounds, pages,
           (unsigned long long)(ended - compute), (unsigned long long)(ended - began), (unsigned long long)sum);
    return 0;
}
