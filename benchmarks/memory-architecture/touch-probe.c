// SPDX-License-Identifier: GPL-3.0-or-later
// Hardware-only EL1 lower bound, not a Linux runtime or production backend.
#include <Hypervisor/Hypervisor.h>
#include <sys/mman.h>
#include <time.h>
#include <stdint.h>
#include <stdlib.h>
#include <stdio.h>
#include <string.h>
#include <assert.h>
#include <libkern/OSCacheControl.h>

// Same store/stride/branch body for the native and hardware-only probe.
#define BODY "1: ubfx x2,x0,#3,#10\n str x2,[x0]\n add x0,x0,#1,lsl #12\n cmp x0,x1\n b.lo 1b\n"
__asm__(".text\n.p2align 2\n.globl _native_loop\n_native_loop:\n" BODY "ret\n"
        ".p2align 2\n.globl _guest_loop_begin\n_guest_loop_begin:\n" BODY "hvc #0x5452\n.globl _guest_loop_end\n_guest_loop_end:\n");
extern void native_loop(void *,void *);
extern const unsigned char guest_loop_begin[],guest_loop_end[];
static double now(void) { struct timespec t; assert(!clock_gettime(CLOCK_MONOTONIC,&t)); return t.tv_sec+t.tv_nsec*1e-9; }
static void *region(size_t len) { void *p=mmap(NULL,len,PROT_READ|PROT_WRITE,MAP_PRIVATE|MAP_ANON,-1,0); assert(p!=MAP_FAILED); return p; }
int main(int argc,char **argv) {
    assert(argc==3); size_t len=(size_t)strtoull(argv[2],NULL,10)<<20; assert(len && len<=(512UL<<20) && !(len%0x200000));
    double start=now(); unsigned char *ram=region(len); double setup,first,steady;
    if(!strcmp(argv[1],"native")) {
        setup=now()-start; double t=now(); native_loop(ram,ram+len); first=now()-t;
        t=now(); native_loop(ram,ram+len); steady=now()-t;
    } else {
        assert(hv_vm_create(NULL)==HV_SUCCESS);
        uint64_t *tables=region(16384); unsigned char *code=region(16384);
        const uint64_t TABLE=0x1000000,CODE=0x800000,DATA=0x100000000,VA=0x40000000;
        tables[0]=(TABLE+4096)|3;
        tables[512]=(TABLE+8192)|3; tables[513]=(TABLE+12288)|3;
        const uint64_t flags=1|(1UL<<10)|(3UL<<8);
        tables[1024+4]=CODE|flags;
        for(size_t i=0;i<len/0x200000;i++)tables[1536+i]=(DATA+i*0x200000)|flags;
        memcpy(code,guest_loop_begin,guest_loop_end-guest_loop_begin);sys_icache_invalidate(code,16384);
        assert(hv_vm_map(tables,TABLE,16384,HV_MEMORY_READ|HV_MEMORY_WRITE)==HV_SUCCESS);
        assert(hv_vm_map(code,CODE,16384,HV_MEMORY_READ|HV_MEMORY_EXEC)==HV_SUCCESS);
        assert(hv_vm_map(ram,DATA,len,HV_MEMORY_READ|HV_MEMORY_WRITE)==HV_SUCCESS);
        hv_vcpu_t cpu;hv_vcpu_exit_t *exit;
        assert(hv_vcpu_create(&cpu,&exit,NULL)==HV_SUCCESS);
        assert(hv_vcpu_set_sys_reg(cpu,HV_SYS_REG_TTBR0_EL1,TABLE)==HV_SUCCESS);
        assert(hv_vcpu_set_sys_reg(cpu,HV_SYS_REG_TCR_EL1,16|(1<<8)|(1<<10)|(3<<12)|(1<<23)|(2UL<<32))==HV_SUCCESS);
        assert(hv_vcpu_set_sys_reg(cpu,HV_SYS_REG_MAIR_EL1,0xff)==HV_SUCCESS);
        assert(hv_vcpu_set_sys_reg(cpu,HV_SYS_REG_SCTLR_EL1,0x30d00800UL|1|4|(1<<12)|(1<<15)|(1<<26))==HV_SUCCESS);
        setup=now()-start;
        for(unsigned pass=0;pass<2;pass++) {
            assert(hv_vcpu_set_reg(cpu,HV_REG_CPSR,0x3c5)==HV_SUCCESS);
            assert(hv_vcpu_set_reg(cpu,HV_REG_PC,CODE)==HV_SUCCESS);
            assert(hv_vcpu_set_reg(cpu,HV_REG_X0,VA)==HV_SUCCESS);
            assert(hv_vcpu_set_reg(cpu,HV_REG_X1,VA+len)==HV_SUCCESS);
            double t=now();assert(hv_vcpu_run(cpu)==HV_SUCCESS);double elapsed=now()-t;
            assert(exit->reason==HV_EXIT_REASON_EXCEPTION && exit->exception.syndrome>>26==0x16);
            if(!pass)first=elapsed;else steady=elapsed;
        }
        assert(hv_vcpu_destroy(cpu)==HV_SUCCESS);
        assert(hv_vm_unmap(DATA,len)==HV_SUCCESS);assert(hv_vm_unmap(TABLE,16384)==HV_SUCCESS);assert(hv_vm_unmap(CODE,16384)==HV_SUCCESS);
        assert(hv_vm_destroy()==HV_SUCCESS);assert(!munmap(tables,16384));assert(!munmap(code,16384));
    }
    uint64_t sum=0;
    for(size_t at=0;at<len;at+=4096) { uint64_t value=*(volatile uint64_t *)(ram+at);assert(value==((at/8)&1023));sum+=value; }
    assert(!munmap(ram,len));
    printf("{\"mode\":\"%s\",\"mib\":%zu,\"setup_ms\":%.6f,\"first_touch_ms\":%.6f,\"retouch_ms\":%.6f,\"checksum\":%llu,\"complete_linux_semantics\":false}\n",argv[1],len>>20,setup*1000,first*1000,steady*1000,(unsigned long long)sum);
}
