// SPDX-License-Identifier: GPL-3.0-or-later
#include <assert.h>
#include <pthread.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <sys/mman.h>
#include <time.h>
static unsigned char *ram;
static size_t bytes;
static unsigned count;
static uint64_t sums[16];
static pthread_barrier_t gate;
static double now(void) {struct timespec t;assert(!clock_gettime(CLOCK_MONOTONIC,&t));return t.tv_sec+t.tv_nsec/1e9;}
static void barrier(void) {int r=pthread_barrier_wait(&gate);assert(!r||r==PTHREAD_BARRIER_SERIAL_THREAD);}
static void *worker(void *arg) {
 unsigned id=(uintptr_t)arg; barrier();
 for(size_t at=id*4096;at<bytes;at+=(size_t)count*4096) *(uint64_t *)(ram+at)=at^0x513a77;
 barrier();barrier();
 uint64_t sum=0;
 for(unsigned round=0;round<8;round++)for(size_t at=id*4096;at<bytes;at+=(size_t)count*4096) {uint64_t *v=(uint64_t *)(ram+at);assert(*v==(at^0x513a77));sum+=*v;}
 sums[id]=sum;barrier();return 0;
}
int main(int argc,char **argv) {
 assert(argc==3);count=atoi(argv[1]);bytes=(size_t)atoi(argv[2])*1024*1024;assert(count&&count<=16&&bytes);
 double begin=now();ram=mmap(0,bytes,PROT_READ|PROT_WRITE,MAP_PRIVATE|MAP_ANONYMOUS,-1,0);assert(ram!=MAP_FAILED);double mapped=now();
 assert(!pthread_barrier_init(&gate,0,count+1));pthread_t threads[16];for(unsigned i=0;i<count;i++)assert(!pthread_create(threads+i,0,worker,(void *)(uintptr_t)i));
 double ready=now();barrier();barrier();double touched=now();barrier();barrier();double reused=now();
 uint64_t sum=0,expected=0;for(unsigned i=0;i<count;i++){assert(!pthread_join(threads[i],0));sum+=sums[i];}for(size_t at=0;at<bytes;at+=4096)expected+=(at^0x513a77)*8;
 assert(sum==expected);assert(!munmap(ram,bytes));double end=now();
 printf("{\"threads\":%u,\"mib\":%zu,\"map_ms\":%.3f,\"startup_ms\":%.3f,\"first_touch_ms\":%.3f,\"reuse_ms\":%.3f,\"retire_ms\":%.3f,\"checksum\":%llu}\n",count,bytes/1024/1024,(mapped-begin)*1000,(ready-mapped)*1000,(touched-ready)*1000,(reused-touched)*1000,(end-reused)*1000,(unsigned long long)sum);return 0;
}
