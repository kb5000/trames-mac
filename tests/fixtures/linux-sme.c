// SPDX-License-Identifier: GPL-3.0-or-later
#define _GNU_SOURCE
#include <assert.h>
#include <errno.h>
#include <pthread.h>
#include <signal.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/auxv.h>
#include <sys/prctl.h>
#include <sys/time.h>
#include <sys/wait.h>
#include <unistd.h>
#define CHECK(x) do { if (!(x)) { fprintf(stderr,"line %d: %s errno=%d\n",__LINE__,#x,errno); _exit(1); } } while(0)
#define ZA_MAGIC 0x54366345
#define ZT_MAGIC 0x5a544e01
#define TPIDR2_MAGIC 0x54504902
#define INHERIT (1UL<<17)
#define ONEXEC (1UL<<18)
static unsigned char alt[65536] __attribute__((aligned(16)));
static volatile sig_atomic_t received;
static unsigned char result[8192] __attribute__((aligned(64)));
void sme_signal_probe(void *,volatile sig_atomic_t *);
void sme_clobber(void);
static void mode(unsigned long *svcr,unsigned long *tls) {
    __asm__ volatile(".arch_extension sme\n mrs %0,svcr\n mrs %1,tpidr2_el0":"=r"(*svcr),"=r"(*tls));
}
static void signal_handler(int sig,siginfo_t *info,void *context) {
    (void)info;CHECK(sig==SIGALRM);
    unsigned long svcr,tls; mode(&svcr,&tls);CHECK(svcr==0 && tls==0);
    unsigned char local;CHECK(&local>=alt && &local<alt+sizeof alt);
    ucontext_t *uc=context;unsigned char *p=(void *)uc->uc_mcontext.__reserved;
    unsigned seen=0;
    for (;;) {
        struct _aarch64_ctx *head=(void *)p;
        if (!head->magic && !head->size) break;
        CHECK(head->size>=16 && !(head->size&15));
        if (head->magic==EXTRA_MAGIC) {
            struct extra_context *extra=(void *)p;
            CHECK(extra->size<=8192 && extra->datap==(uintptr_t)(p+48));
            p=(void *)extra->datap;continue;
        }
        if (head->magic==SVE_MAGIC) {
            unsigned short *h=(void *)(p+8);CHECK(h[0]==64 && h[1]==1);seen|=1;
        } else if (head->magic==ZA_MAGIC) {
            CHECK(*(unsigned short *)(p+8)==64 && head->size==4112);seen|=2;
        } else if (head->magic==ZT_MAGIC) {
            CHECK(*(unsigned short *)(p+8)==1 && head->size==80);seen|=4;
        } else if (head->magic==TPIDR2_MAGIC) {
            CHECK(*(unsigned long *)(p+8)==0x713);seen|=8;
        }
        p+=head->size;
    }
    CHECK(seen==15);sme_clobber();received=1;
}
static void *thread(void *unused) {
    (void)unused;unsigned long svcr,tls;mode(&svcr,&tls);CHECK(svcr==0 && tls==0);
    CHECK(prctl(64,0UL,0UL,0UL,0UL)==64);return NULL;
}
int main(int argc,char **argv) {
    unsigned long svcr,tls;
    CHECK(getauxval(51)>=7600);
    CHECK(getauxval(26)&(1UL<<23));CHECK(getauxval(26)&(1UL<<37));
    if (argc==2) {
        CHECK(!strcmp(argv[1],"exec"));mode(&svcr,&tls);CHECK(svcr==0 && tls==0);
        CHECK(prctl(64,0UL,0UL,0UL,0UL)==(64|INHERIT));return 0;
    }
    CHECK(prctl(64,7UL,8UL,9UL,10UL)==64); // Linux ignores unused arguments.
    CHECK(prctl(63,64UL,8UL,9UL,10UL)==64);
    for (unsigned long n=16;n<=96;n+=16) CHECK(prctl(63,n,0UL,0UL,0UL)==64);
    CHECK(prctl(63,8192UL,0UL,0UL,0UL)==64);
    CHECK(prctl(63,15UL,0UL,0UL,0UL)==-1 && errno==EINVAL);
    CHECK(prctl(63,64|INHERIT|ONEXEC,0UL,0UL,0UL)==(64|INHERIT));
    pid_t pid=fork();CHECK(pid>=0);
    if (!pid) { execl("/proc/self/exe",argv[0],"exec",NULL);execl("/program",argv[0],"exec",NULL);_exit(2); }
    int status;CHECK(waitpid(pid,&status,0)==pid && status==0);
    CHECK(prctl(63,64UL,0UL,0UL,0UL)==64);
    __asm__ volatile(".arch_extension sme\n smstart za\n mov x9,#0x713\n msr tpidr2_el0,x9":::"x9","memory");
    pid=fork();CHECK(pid>=0);
    if (!pid) { mode(&svcr,&tls);_exit(svcr==2 && tls==0x713 ? 0 : 3); }
    CHECK(waitpid(pid,&status,0)==pid && status==0);
    pthread_t t;CHECK(!pthread_create(&t,NULL,thread,NULL));CHECK(!pthread_join(t,NULL));
    mode(&svcr,&tls);CHECK(svcr==2 && tls==0x713);
    __asm__ volatile(".arch_extension sme\n smstop\n msr tpidr2_el0,xzr":::"memory");
    stack_t stack={.ss_sp=alt,.ss_size=sizeof alt};CHECK(!sigaltstack(&stack,NULL));
    struct sigaction sa={.sa_sigaction=signal_handler,.sa_flags=SA_SIGINFO|SA_ONSTACK};sigemptyset(&sa.sa_mask);CHECK(!sigaction(SIGALRM,&sa,NULL));
    memset(result,0xa7,64);
    struct itimerval timer={.it_value={.tv_usec=20000}};CHECK(!setitimer(ITIMER_REAL,&timer,NULL));
    sme_signal_probe(result,&received);CHECK(received);
    CHECK(*(unsigned long *)(result+4224)==3 && *(unsigned long *)(result+4232)==0x713);
    unsigned count=0;
    for (unsigned i=0;i<4096;i+=4) { float x;memcpy(&x,result+128+i,4);if(x){CHECK(x==4.0f);count++;} }
    CHECK(count==256);
    for (unsigned i=0;i<64;i++) CHECK(result[64+i]==0xa7);
    puts("SME: VL, exec, fork, CLONE_VM, streaming signal frame, ZA/ZT/TLS restoration passed");
    return 0;
}
