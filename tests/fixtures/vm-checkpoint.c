// SPDX-License-Identifier: GPL-3.0-or-later
// Fixed CPU work runs beside mapping changes. No wall-time assertion or spin
// protocol: a pthread barrier starts the jobs and each computes finite work.
#define _GNU_SOURCE
#include <assert.h>
#include <pthread.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <sys/mman.h>
#include <time.h>
static pthread_barrier_t start;
static unsigned peers, rounds;
static uint64_t iterations;
struct result { double ms; uint64_t value; } __attribute__((aligned(64))) results[32];
static double now(void) { struct timespec t; assert(!clock_gettime(CLOCK_MONOTONIC,&t)); return t.tv_sec*1e3+t.tv_nsec*1e-6; }
static void barrier(void) { int r=pthread_barrier_wait(&start);assert(!r || r==PTHREAD_BARRIER_SERIAL_THREAD); }
static void *compute(void *arg) {
    uintptr_t id=(uintptr_t)arg;barrier();double began=now();uint64_t x=id+1;
    for (uint64_t i=0;i<iterations;i++) { x^=x<<13;x^=x>>7;x^=x<<17; }
    results[id].ms=now()-began;results[id].value=x;return NULL;
}
int main(int argc,char **argv) {
    assert(argc==4);peers=atoi(argv[1]);rounds=atoi(argv[2]);iterations=strtoull(argv[3],NULL,10);
    assert(peers<=31 && rounds && iterations);assert(!pthread_barrier_init(&start,NULL,peers+1));
    pthread_t threads[32];for (uintptr_t i=0;i<peers;i++) assert(!pthread_create(threads+i,NULL,compute,(void *)i));
    barrier();double began=now();uint64_t sum=0;
    for (unsigned i=0;i<rounds;i++) {
        volatile uint64_t *p=mmap(NULL,4096,PROT_READ|PROT_WRITE,MAP_PRIVATE|MAP_ANONYMOUS,-1,0);assert(p!=MAP_FAILED);
        *p=i+1;assert(!mprotect((void *)p,4096,PROT_READ));assert(*p==i+1);sum+=*p;
        assert(!mprotect((void *)p,4096,PROT_READ|PROT_WRITE));*p=~(uint64_t)i;
        assert(!munmap((void *)p,4096));
    }
    double mapping=now()-began;double compute_ms=0;
    for (unsigned i=0;i<peers;i++) {assert(!pthread_join(threads[i],NULL));assert(results[i].value);sum^=results[i].value;compute_ms+=results[i].ms;}
    printf("{\"peers\":%u,\"rounds\":%u,\"iterations\":%llu,\"mapping_ms\":%.3f,\"compute_sum_ms\":%.3f,\"wall_ms\":%.3f,\"checksum\":%llu,\"values\":[",peers,rounds,(unsigned long long)iterations,mapping,compute_ms,now()-began,(unsigned long long)sum);
    for (unsigned i=0;i<peers;i++) printf("%s%llu",i?",":"",(unsigned long long)results[i].value);
    puts("]}");
    return 0;
}
