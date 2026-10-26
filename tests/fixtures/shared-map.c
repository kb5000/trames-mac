// SPDX-License-Identifier: GPL-3.0-or-later
#include <assert.h>
#include <fcntl.h>
#include <stdatomic.h>
#include <stdio.h>
#include <sys/mman.h>
#include <sys/wait.h>
#include <unistd.h>
int main(void) {
    int fd=open("/tmp/trames-shared-map",O_CREAT|O_TRUNC|O_RDWR,0600);assert(fd>=0);assert(!ftruncate(fd,4096));
    atomic_uint *count=mmap(0,4096,PROT_READ|PROT_WRITE,MAP_SHARED,fd,0);assert(count!=MAP_FAILED);atomic_init(count,0);
    pid_t child=fork();assert(child>=0);
    for(unsigned i=0;i<100000;i++) {
        atomic_fetch_add_explicit(count,1,memory_order_relaxed);
        if(child && i%1000==0){unsigned sample;assert(pread(fd,&sample,sizeof sample,0)==sizeof sample);assert(sample<=200000);}
    }
    if(!child)_exit(21);
    int status;assert(waitpid(child,&status,0)==child && WIFEXITED(status) && WEXITSTATUS(status)==21);
    assert(atomic_load(count)==200000);assert(!msync(count,4096,MS_SYNC));
    unsigned sample;assert(pread(fd,&sample,sizeof sample,0)==sizeof sample);assert(sample==200000);
    assert(!munmap(count,4096));close(fd);assert(!unlink("/tmp/trames-shared-map"));
    puts("MAP_SHARED fork atomics and concurrent VFS pread: ok");return 0;
}
