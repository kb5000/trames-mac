// SPDX-License-Identifier: GPL-3.0-or-later
#include <assert.h>
#include <pthread.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <time.h>
static unsigned count, rounds, reuse, sparse;
static size_t chunk;
static pthread_mutex_t lock=PTHREAD_MUTEX_INITIALIZER;
static pthread_cond_t changed=PTHREAD_COND_INITIALIZER;
static unsigned ready, go;
struct times {double map,touch,read,unmap;uint64_t sum;} times[32];
static double now(void){struct timespec ts;assert(!clock_gettime(CLOCK_MONOTONIC,&ts));return ts.tv_sec+ts.tv_nsec*1e-9;}
static unsigned char *allocate(void){void *p=mmap(0,chunk,PROT_READ|PROT_WRITE,MAP_PRIVATE|MAP_ANONYMOUS,-1,0);assert(p!=MAP_FAILED);return p;}
static void *worker(void *arg){unsigned id=(uintptr_t)arg;struct times *t=times+id;
 assert(!pthread_mutex_lock(&lock));ready++;assert(!pthread_cond_broadcast(&changed));while(!go)assert(!pthread_cond_wait(&changed,&lock));assert(!pthread_mutex_unlock(&lock));
 unsigned char *p=0;size_t stride=sparse?4096:8;
 for(unsigned round=0;round<rounds;round++){
  double a=now();if(!reuse||!p)p=allocate();double b=now();
  for(size_t at=0;at<chunk;at+=stride)*(volatile uint64_t *)(p+at)=((at/8)&1023)+round;
  double c=now();uint64_t sum=0;
  for(size_t at=0;at<chunk;at+=stride){uint64_t value=*(volatile uint64_t *)(p+at);assert(value==((at/8)&1023)+round);sum+=value;}
  double d=now();if(!reuse||round==rounds-1){assert(!munmap(p,chunk));p=0;}double e=now();
  t->map+=b-a;t->touch+=c-b;t->read+=d-c;t->unmap+=e-d;t->sum+=sum;
 }return 0;}
int main(int argc,char **argv){assert(argc==6);count=atoi(argv[1]);chunk=(size_t)atoi(argv[2])<<20;rounds=atoi(argv[3]);reuse=atoi(argv[4]);sparse=atoi(argv[5]);assert(count>0&&count<=32&&chunk&&rounds);
 pthread_t threads[32];for(unsigned i=0;i<count;i++)assert(!pthread_create(threads+i,0,worker,(void *)(uintptr_t)i));
 assert(!pthread_mutex_lock(&lock));while(ready!=count)assert(!pthread_cond_wait(&changed,&lock));double begin=now();go=1;assert(!pthread_cond_broadcast(&changed));assert(!pthread_mutex_unlock(&lock));
 struct times total={0};for(unsigned i=0;i<count;i++){assert(!pthread_join(threads[i],0));total.map+=times[i].map;total.touch+=times[i].touch;total.read+=times[i].read;total.unmap+=times[i].unmap;total.sum+=times[i].sum;}
 printf("{\"threads\":%u,\"chunk_mib\":%zu,\"rounds\":%u,\"reuse\":%u,\"sparse\":%u,\"wall_ms\":%.3f,\"map_thread_ms\":%.3f,\"touch_thread_ms\":%.3f,\"read_thread_ms\":%.3f,\"unmap_thread_ms\":%.3f,\"checksum\":%llu}\n",count,chunk>>20,rounds,reuse,sparse,(now()-begin)*1000,total.map*1000,total.touch*1000,total.read*1000,total.unmap*1000,(unsigned long long)total.sum);return 0;}
