// SPDX-License-Identifier: GPL-3.0-or-later
#define _GNU_SOURCE
#include <assert.h>
#include <elf.h>
#include <errno.h>
#include <pthread.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/auxv.h>
#include <sys/syscall.h>
#include <sys/time.h>
#include <sys/wait.h>
#include <time.h>
#include <unistd.h>
static uint64_t stamp(struct timespec t) { return (uint64_t)t.tv_sec*1000000000+t.tv_nsec; }
static void *symbol(const char *wanted) {
    uintptr_t base=getauxval(AT_SYSINFO_EHDR);if (!base)return NULL;
    const Elf64_Ehdr *eh=(const void *)base;assert(!memcmp(eh->e_ident,"\177ELF",4) && eh->e_machine==EM_AARCH64);
    const Elf64_Phdr *ph=(const void *)(base+eh->e_phoff);const Elf64_Dyn *dy=NULL;
    for (int i=0;i<eh->e_phnum;i++) if(ph[i].p_type==PT_DYNAMIC)dy=(const void *)(base+ph[i].p_vaddr);
    assert(dy);const Elf64_Sym *symbols=NULL;const char *strings=NULL;const uint32_t *hash=NULL;const uint16_t *versions=NULL;const Elf64_Verdef *def=NULL;
    for (;dy->d_tag;dy++) switch(dy->d_tag) {
      case DT_SYMTAB:symbols=(const void *)(base+dy->d_un.d_ptr);break;
      case DT_STRTAB:strings=(const void *)(base+dy->d_un.d_ptr);break;
      case DT_HASH:hash=(const void *)(base+dy->d_un.d_ptr);break;
      case DT_VERSYM:versions=(const void *)(base+dy->d_un.d_ptr);break;
      case DT_VERDEF:def=(const void *)(base+dy->d_un.d_ptr);break;
    }
    assert(symbols && strings && hash && versions && def);
    const Elf64_Verdaux *aux=(const void *)((const char *)def+def->vd_aux);assert(!strcmp(strings+aux->vda_name,"LINUX_2.6.39"));
    for (uint32_t i=1;i<hash[1];i++) if(!strcmp(strings+symbols[i].st_name,wanted)) {assert(versions[i]==def->vd_ndx);return (void *)(base+symbols[i].st_value);}
    assert(!"missing vDSO symbol");return NULL;
}
static void *thread_clock(void *arg) {
    (void)arg;uint64_t previous=0;
    for(int i=0;i<10000;i++) {struct timespec t;assert(!clock_gettime(CLOCK_MONOTONIC,&t));uint64_t n=stamp(t);assert(n>=previous);previous=n;if(!(i%1000))sched_yield();}
    return NULL;
}
int main(int argc,char **argv) {
    if(argc>1 && !strcmp(argv[1],"--bench")) {
        uint64_t count=argc>2?strtoull(argv[2],NULL,10):1000000, sum=0;struct timespec a,b,t;
        assert(!clock_gettime(CLOCK_MONOTONIC,&a));for(uint64_t i=0;i<count;i++){assert(!clock_gettime(CLOCK_MONOTONIC,&t));sum+=t.tv_nsec;}
        assert(!clock_gettime(CLOCK_MONOTONIC,&b));printf("{\"vdso\":%s,\"calls\":%llu,\"elapsed_ns\":%llu,\"checksum\":%llu}\n",getauxval(AT_SYSINFO_EHDR)?"true":"false",(unsigned long long)count,(unsigned long long)(stamp(b)-stamp(a)),(unsigned long long)sum);return 0;
    }
    assert(getauxval(AT_SYSINFO_EHDR));
    int (*fast)(clockid_t,struct timespec *)=symbol("__kernel_clock_gettime");
    int (*resolution)(clockid_t,struct timespec *)=symbol("__kernel_clock_getres");
    int (*wall)(struct timeval *,void *)=symbol("__kernel_gettimeofday");
    assert(symbol("__kernel_rt_sigreturn"));
    for(int id=0;id<=7;id++) {
        if(id==2 || id==3)continue;
        struct timespec r={7,7};assert(!resolution(id,&r) && !r.tv_sec && r.tv_nsec==1);assert(!resolution(id,NULL));
        for(int i=0;i<1000;i++){struct timespec a,b,c;assert(!syscall(SYS_clock_gettime,id,&a));assert(!fast(id,&b));assert(!syscall(SYS_clock_gettime,id,&c));assert(stamp(a)<=stamp(b) && stamp(b)<=stamp(c));}
    }
    struct timespec t;assert(fast(-1,&t)==-EINVAL);assert(resolution(-1,&t)==-EINVAL);
    errno=0;assert(clock_gettime(-1,&t)==-1 && errno==EINVAL);
    struct timeval tv;assert(!wall(&tv,NULL) && tv.tv_sec>1700000000);
    if(argc>1 && !strcmp(argv[1],"--exec")){puts("clock-exec-ok");return 0;}
    pthread_t threads[12];for(int i=0;i<12;i++)assert(!pthread_create(threads+i,NULL,thread_clock,NULL));for(int i=0;i<12;i++)assert(!pthread_join(threads[i],NULL));
    pid_t pid=fork();assert(pid>=0);if(!pid){execl("/proc/self/exe",argv[0],"--exec",NULL);execl("/program",argv[0],"--exec",NULL);_exit(2);}int status;assert(waitpid(pid,&status,0)==pid && WIFEXITED(status) && !WEXITSTATUS(status));
    puts("clock: ELF/versioned symbols, libc, syscall agreement, threads, fork/exec passed");return 0;
}
