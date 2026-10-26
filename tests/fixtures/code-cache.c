// SPDX-License-Identifier: GPL-3.0-or-later
#include <assert.h>
#include <stdint.h>
#include <stdio.h>
#include <sys/mman.h>
int main(void) {
    uint32_t *code=mmap(0,4096,PROT_READ|PROT_WRITE|PROT_EXEC,MAP_PRIVATE|MAP_ANONYMOUS,-1,0);assert(code!=MAP_FAILED);
    code[0]=0x528000e0;code[1]=0xd65f03c0;
    __builtin___clear_cache((char *)code,(char *)(code+2));
    unsigned (*call)(void)=(unsigned (*)(void))code;assert(call()==7);
    code[0]=0x52800540;__builtin___clear_cache((char *)code,(char *)(code+2));assert(call()==42);
    assert(!mprotect(code,4096,PROT_READ|PROT_WRITE));code[0]=0x52800160;
    assert(!mprotect(code,4096,PROT_READ|PROT_EXEC));__builtin___clear_cache((char *)code,(char *)(code+2));assert(call()==11);
    assert(!munmap(code,4096));puts("EL0 code-cache maintenance and W^X: ok");return 0;
}
