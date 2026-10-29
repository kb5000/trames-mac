// SPDX-License-Identifier: GPL-3.0-or-later
#define _GNU_SOURCE
#include <assert.h>
#include <errno.h>
#define FUTEX_WAIT_PRIVATE 128
#define FUTEX_WAKE_PRIVATE 129
#include <pthread.h>
#include <stdatomic.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <sys/syscall.h>
#include <time.h>
#include <unistd.h>
static struct { _Atomic unsigned token; uint64_t turns; } __attribute__((aligned(64))) slots[32];
static pthread_barrier_t start;
static unsigned threads,rounds;
static uint64_t now(void){struct timespec t;assert(!clock_gettime(CLOCK_MONOTONIC,&t));return (uint64_t)t.tv_sec*1000000000+t.tv_nsec;}
static void barrier(void){int r=pthread_barrier_wait(&start);assert(!r||r==PTHREAD_BARRIER_SERIAL_THREAD);}
static void wake(unsigned id){atomic_store_explicit(&slots[id].token,1,memory_order_release);assert(syscall(SYS_futex,&slots[id].token,FUTEX_WAKE_PRIVATE,1)>=0);}
static void *run(void *arg){unsigned id=(uintptr_t)arg;barrier();for(unsigned i=0;i<rounds;i++){while(!atomic_load_explicit(&slots[id].token,memory_order_acquire)){long r=syscall(SYS_futex,&slots[id].token,FUTEX_WAIT_PRIVATE,0,NULL);assert(!r||errno==EAGAIN||errno==EINTR);}atomic_store_explicit(&slots[id].token,0,memory_order_relaxed);slots[id].turns++;wake((id+1)%threads);}return NULL;}
int main(int argc,char **argv){assert(argc==3);threads=atoi(argv[1]);rounds=atoi(argv[2]);assert(threads>=2&&threads<=32&&rounds);pthread_t jobs[32];assert(!pthread_barrier_init(&start,NULL,threads+1));for(uintptr_t i=0;i<threads;i++)assert(!pthread_create(jobs+i,NULL,run,(void *)i));barrier();uint64_t began=now();wake(0);uint64_t sum=0;for(unsigned i=0;i<threads;i++){assert(!pthread_join(jobs[i],NULL));assert(slots[i].turns==rounds);sum+=slots[i].turns;}printf("{\"threads\":%u,\"rounds\":%u,\"elapsed_ns\":%llu,\"checksum\":%llu}\n",threads,rounds,(unsigned long long)(now()-began),(unsigned long long)sum);return 0;}
