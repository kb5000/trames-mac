// SPDX-License-Identifier: GPL-3.0-or-later
#include <assert.h>
#include <stdint.h>
#include <stdio.h>
#include <sys/auxv.h>
int main(void) {
    unsigned long cap = getauxval(16), cap2 = getauxval(26);
    printf("hwcap=%lx hwcap2=%lx\n", cap, cap2);
    assert((cap & 3) == 3);
    assert(!(cap & (1ul << 22))); // SVE state is not advertised.
    assert(!(cap2 & (1ul << 30))); // FA64 state is not advertised.
    uint32_t result[4];
    if (cap & (1ul << 3)) {
        __asm__ volatile(".arch_extension crypto\n movi v0.16b,#0\n movi v1.16b,#0\n aese v0.16b,v1.16b\n str q0,[%0]" :: "r"(result) : "v0","v1","memory");
        for (unsigned i=0;i<4;i++) assert(result[i]==0x63636363);
        puts("aes ok");
    }
    if (cap & (1ul << 4)) {
        uint64_t left[2]={1,0},right[2]={2,0};
        __asm__ volatile(".arch_extension crypto\n ldr q0,[%1]\n ldr q1,[%2]\n pmull v2.1q,v0.1d,v1.1d\n str q2,[%0]" :: "r"(result),"r"(left),"r"(right) : "v0","v1","v2","memory");
        assert(result[0]==2 && result[1]==0 && result[2]==0 && result[3]==0);
        puts("pmull ok");
    }
    if (cap & (1ul << 5)) {
        uint32_t value;
        __asm__ volatile(".arch_extension crypto\n movi v1.16b,#0\n sha1h s0,s1\n fmov %w0,s0" : "=r"(value) :: "v0","v1");
        assert(value==0); puts("sha1 ok");
    }
    if (cap & (1ul << 6)) {
        __asm__ volatile(".arch_extension crypto\n movi v0.16b,#0\n movi v1.16b,#0\n movi v2.16b,#0\n sha256h q0,q1,v2.4s\n str q0,[%0]" :: "r"(result) : "v0","v1","v2","memory");
        for(unsigned i=0;i<4;i++) assert(!result[i]);
        puts("sha2 ok");
    }
    if (cap & (1ul << 7)) {
        uint32_t value;
        __asm__ volatile(".arch_extension crc\n crc32x %w0,wzr,xzr" : "=r"(value));
        assert(!value); puts("crc32 ok");
    }
    if (cap & (1ul << 8)) {
        uint64_t memory=5,old;
        __asm__ volatile(".arch_extension lse\n mov x1,#7\n ldadd x1,%0,[%1]" : "=&r"(old) : "r"(&memory) : "x1","memory");
        assert(old==5 && memory==12); puts("lse atomics ok");
    }
    if (cap & (1ul << 10)) {
        uint16_t a[8],b[8]; for(unsigned i=0;i<8;i++){a[i]=0x3c00;b[i]=0x4000;}
        __asm__ volatile(".arch_extension fp16\n ldr q0,[%1]\n ldr q1,[%2]\n fadd v2.8h,v0.8h,v1.8h\n str q2,[%0]" :: "r"(result),"r"(a),"r"(b) : "v0","v1","v2","memory");
        for(unsigned i=0;i<4;i++)assert(result[i]==0x42004200);
        puts("neon fp16 ok");
    }
    if (cap & (1ul << 12)) {
        __asm__ volatile(".arch_extension rdm\n movi v0.16b,#0\n movi v1.16b,#0\n movi v2.16b,#0\n sqrdmlah v0.4s,v1.4s,v2.4s\n str q0,[%0]" :: "r"(result) : "v0","v1","v2","memory");
        for(unsigned i=0;i<4;i++)assert(!result[i]); puts("rdm ok");
    }
    if (cap & (1ul << 20)) {
        __asm__ volatile(".arch_extension dotprod\n movi v0.16b,#1\n movi v1.16b,#2\n movi v2.4s,#0\n sdot v2.4s,v0.16b,v1.16b\n str q2,[%0]" :: "r"(result) : "v0","v1","v2","memory");
        for(unsigned i=0;i<4;i++)assert(result[i]==8); puts("dotprod ok");
    }
    if (cap & (1ul << 23)) {
        __asm__ volatile(".arch_extension fp16fml\n movi v0.16b,#0\n movi v1.16b,#0\n movi v2.16b,#0\n fmlal v0.4s,v1.4h,v2.4h\n str q0,[%0]" :: "r"(result) : "v0","v1","v2","memory");
        for(unsigned i=0;i<4;i++)assert(!result[i]); puts("fhm ok");
    }
    if (cap2 & (1ul << 13)) {
        __asm__ volatile(".arch_extension i8mm\n movi v0.16b,#1\n movi v1.16b,#2\n movi v2.4s,#0\n smmla v2.4s,v0.16b,v1.16b\n str q2,[%0]" :: "r"(result) : "v0","v1","v2","memory");
        for(unsigned i=0;i<4;i++)assert(result[i]==16); puts("i8mm ok");
    }
    if (cap2 & (1ul << 14)) {
        uint16_t a[8],b[8];for(unsigned i=0;i<8;i++){a[i]=0x3f80;b[i]=0x4000;}
        __asm__ volatile(".arch_extension bf16\n ldr q0,[%1]\n ldr q1,[%2]\n movi v2.4s,#0\n bfdot v2.4s,v0.8h,v1.8h\n str q2,[%0]" :: "r"(result),"r"(a),"r"(b) : "v0","v1","v2","memory");
        for(unsigned i=0;i<4;i++)assert(result[i]==0x40800000); puts("bf16 ok");
    }
    puts("advertised instruction subset ok");
}
