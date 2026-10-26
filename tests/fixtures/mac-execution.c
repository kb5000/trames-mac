// SPDX-License-Identifier: GPL-3.0-or-later
// Original static musl/native ARM64 execution; no Trames-specific instructions.
#define _GNU_SOURCE
#include <assert.h>
#include <fcntl.h>
#include <errno.h>
#include <sys/stat.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>
#include <sys/mman.h>
#include <unistd.h>
static int interactive;
static void marker(const char *name) {
    puts(name); fflush(stdout);
    if (interactive) { char input; assert(read(0,&input,1)==1); }
}
int main(int argc,char **argv) {
    interactive=argc>1 && !strcmp(argv[1],"--markers");
    assert(getpid()>0);
    const size_t length=64*1024*1024;
    marker("BASELINE");
    for (unsigned cycle=1;cycle<=3;cycle++) {
        volatile uint64_t *bytes=mmap(NULL,length,PROT_READ|PROT_WRITE,MAP_PRIVATE|MAP_ANONYMOUS,-1,0);
        assert(bytes!=MAP_FAILED);
        for (size_t offset=0;offset<length;offset+=4096) bytes[offset/8]=offset+cycle;
        marker("POPULATED");
        assert(!madvise((void *)bytes,length,MADV_DONTNEED));
        marker("DISCARDED");
        for (size_t offset=0;offset<length;offset+=1024*1024) assert(bytes[offset/8]==0);
        bytes[0]=1234;
        assert(!munmap((void *)bytes,length));
        marker("UNMAPPED");
    }
    assert(!mkdir("/tmp",0777) || errno==EEXIST);
    int fd=open("/tmp/native-map",O_CREAT|O_RDWR|O_TRUNC,0600);assert(fd>=0);
    assert(write(fd,"original",8)==8);assert(!ftruncate(fd,4096));
    char *private=mmap(NULL,4096,PROT_READ|PROT_WRITE,MAP_PRIVATE,fd,0);assert(private!=MAP_FAILED);
    assert(!memcmp(private,"original",8));private[0]='X';
    char original[8];assert(pread(fd,original,8,0)==8);assert(!memcmp(original,"original",8));
    assert(!madvise(private,4096,MADV_DONTNEED));assert(!memcmp(private,"original",8));
    assert(!munmap(private,4096));assert(!close(fd));
    puts("PASS: demand paging, discard, zero refill, repeated unmap and private file COW");
    return 0;
}
