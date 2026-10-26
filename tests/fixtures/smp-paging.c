// SPDX-License-Identifier: GPL-3.0-or-later
#include <assert.h>
#include <pthread.h>
#include <stdatomic.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <sys/mman.h>
#include <sys/wait.h>
#include <unistd.h>
#define SIZE (64u<<20)
static unsigned char *memory;
static atomic_uint counter;
static pthread_barrier_t barrier;
static unsigned char value(unsigned p, unsigned at) { return (p * 37u + at * 13u + 71u) % 251u; }
static void *worker(void *arg) {
 unsigned id=(uintptr_t)arg;
 int r=pthread_barrier_wait(&barrier); assert(r==0 || r==PTHREAD_BARRIER_SERIAL_THREAD);
 for(unsigned i=0;i<1000000;i++) atomic_fetch_add_explicit(&counter,1,memory_order_relaxed);
 for(unsigned p=id;p<SIZE/4096;p+=4)
  for(unsigned at=0;at<4096;at++) memory[p*4096+at]=value(p,at);
 return 0;
}
static int interactive;
static void marker(const char *name){if(interactive){puts(name);fflush(stdout);assert(getchar()!=EOF);}}
int main(int argc,char **argv) {
 (void)argv;interactive=argc>1;marker("BASELINE");
 memory=mmap(0,SIZE,PROT_READ|PROT_WRITE,MAP_PRIVATE|MAP_ANONYMOUS,-1,0);assert(memory!=MAP_FAILED);
 assert(!pthread_barrier_init(&barrier,0,4));pthread_t t[4];
 for(unsigned i=0;i<4;i++)assert(!pthread_create(&t[i],0,worker,(void *)(uintptr_t)i));
 for(unsigned i=0;i<4;i++)assert(!pthread_join(t[i],0));
 assert(atomic_load(&counter)==4000000);puts("four-thread ARM atomics: ok");fflush(stdout);
 marker("POPULATED");
 uint64_t sum=0;
 for(int round=0;round<2;round++) {
  for(unsigned p=0;p<SIZE/4096;p++)for(unsigned at=0;at<4096;at++){if(memory[p*4096+at]!=value(p,at)){printf("bad page=%u offset=%u got=%u expected=%u\n",p,at,memory[p*4096+at],value(p,at));fflush(stdout);return 81;}sum+=memory[p*4096+at];}
  printf("64MiB paging checksum round %d: %llu\n",round,(unsigned long long)sum);fflush(stdout);
 }
 marker("VERIFIED");
 pid_t child=fork();assert(child>=0);
 if(!child){for(unsigned p=0;p<SIZE/4096;p++)memory[p*4096]^=255;_exit(17);}
 int status;assert(waitpid(child,&status,0)==child && WEXITSTATUS(status)==17);
 for(unsigned p=0;p<SIZE/4096;p++)if(memory[p*4096]!=value(p,0)){printf("COW bad page=%u got=%u expected=%u\n",p,memory[p*4096],value(p,0));return 82;}
 puts("paged fork COW: ok");fflush(stdout);
 assert(!munmap(memory,SIZE));marker("UNMAPPED");puts("done");return 0;
}
