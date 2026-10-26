// SPDX-License-Identifier: GPL-3.0-or-later
// Small EL1 exception/page-table adapter. Linux remains in the host runtime.
#include <Hypervisor/Hypervisor.h>
#include <libkern/OSCacheControl.h>
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <unistd.h>

#define VECTOR_GPA UINT64_C(0x800000)
#define VECTOR_VA UINT64_C(0xffffffE00000)
#define TABLE_GPA UINT64_C(0x1000000)
#define ADDRESS_MASK UINT64_C(0x0000fffffffff000)
#define PAGE 4096

struct state {
    uint64_t x[31], sp, pc;
    uint32_t nzcv;
    uint64_t tpidr, tpidr2;
    uint32_t fpcr, fpsr;
    __uint128_t vectors[32];
};
_Static_assert(sizeof(struct state) == 816, "shared AArch64 state layout");
_Static_assert(offsetof(struct state, vectors) == 304, "shared SIMD offset");
struct result { uint64_t kind, syndrome, address, pc; };
struct table { void *memory; uint64_t gpa; struct table *next; };
struct host {
    hv_vcpu_t cpu;
    hv_vcpu_exit_t *exit;
    bool cpu_created, tlb_dirty;
    size_t granule;
    void *vectors;
    struct table *tables;
    uint64_t *root, root_gpa, next_table;
};
extern const char trames_hvf_vectors_begin[], trames_hvf_vectors_end[];
extern const char trames_hvf_tlbi_begin[], trames_hvf_tlbi_end[];
extern const char trames_hvf_resume_begin[], trames_hvf_resume_end[];
static uint64_t ticks(void) { uint64_t n; __asm__ volatile("mrs %0,cntpct_el0":"=r"(n)); return n; }
static uint64_t frequency(void) { uint64_t n; __asm__ volatile("mrs %0,cntfrq_el0":"=r"(n)); return n; }
static void *anonymous(size_t length) {
    void *p = mmap(NULL, length, PROT_READ|PROT_WRITE, MAP_PRIVATE|MAP_ANON, -1, 0);
    return p == MAP_FAILED ? NULL : p;
}
static uint64_t *table_for(struct host *h, uint64_t gpa) {
    for (struct table *t=h->tables; t; t=t->next)
        if (gpa >= t->gpa && gpa < t->gpa+h->granule)
            return (uint64_t *)((char *)t->memory + gpa-t->gpa);
    return NULL;
}
static uint64_t *new_table(struct host *h, uint64_t *gpa) {
    struct table *t=calloc(1,sizeof(*t));
    if (!t) return NULL;
    t->memory=anonymous(h->granule);
    if (!t->memory) { free(t); return NULL; }
    t->gpa=h->next_table; h->next_table+=h->granule;
    if (hv_vm_map(t->memory,t->gpa,h->granule,HV_MEMORY_READ|HV_MEMORY_WRITE)) {
        munmap(t->memory,h->granule); free(t); return NULL;
    }
    t->next=h->tables; h->tables=t; *gpa=t->gpa;
    return t->memory;
}
static uint64_t *leaf(struct host *h, uint64_t va, bool create) {
    uint64_t *table=h->root;
    for (unsigned level=0; level<3; level++) {
        unsigned index=(unsigned)((va>>(39-level*9))&511);
        if (!(table[index]&1)) {
            if (!create) return NULL;
            uint64_t gpa; if (!new_table(h,&gpa)) return NULL;
            table[index]=gpa|3;
        }
        table=table_for(h,table[index]&ADDRESS_MASK);
        if (!table) return NULL;
    }
    return &table[(va>>12)&511];
}
static int remove_page(struct host *h, uint64_t va) {
    uint64_t *chain[4]={h->root}, gpas[4]={h->root_gpa}; unsigned indexes[3];
    for (unsigned level=0; level<3; level++) {
        indexes[level]=(unsigned)((va>>(39-level*9))&511);
        uint64_t descriptor=chain[level][indexes[level]];
        if (!(descriptor&1)) return 0;
        gpas[level+1]=descriptor&ADDRESS_MASK;
        chain[level+1]=table_for(h,gpas[level+1]); if (!chain[level+1]) return -1;
    }
    chain[3][(va>>12)&511]=0; h->tlb_dirty=true;
    for (unsigned level=3;level;level--) {
        bool empty=true;
        for (unsigned i=0;i<512;i++) if (chain[level][i]) {empty=false;break;}
        if (!empty) break;
        chain[level-1][indexes[level-1]]=0;
        struct table **link=&h->tables;
        while (*link && (*link)->gpa!=gpas[level]) link=&(*link)->next;
        if (!*link) return -1;
        struct table *t=*link;
        hv_return_t e=hv_vm_unmap(t->gpa,h->granule); if (e) return (int)e;
        *link=t->next;munmap(t->memory,h->granule);free(t);
    }
    return 0;
}
int trames_hvf_page(struct host *h, uint64_t va, uint64_t gpa, unsigned flags) {
    if (!flags) return remove_page(h,va);
    uint64_t *p=leaf(h,va,flags!=0);
    if (!p) return flags ? -1 : 0;
    if (!flags) *p=0;
    else {
        // flags: read=1, write=2, execute=4, EL1-only=8.
        uint64_t ap=(flags&8) ? 0 : (flags&2) ? 1 : 3;
        *p=(gpa&ADDRESS_MASK)|3|(UINT64_C(1)<<10)|(UINT64_C(3)<<8)|(ap<<6);
        if (!(flags&8)) *p|=UINT64_C(1)<<53;
        if (!(flags&4) || (flags&8)) *p|=UINT64_C(1)<<54;
        if ((flags&8) && (flags&4)) *p&=~(UINT64_C(1)<<53);
    }
    h->tlb_dirty=true; return 0;
}
int trames_hvf_map(struct host *h, void *memory, uint64_t gpa, size_t size) {
    // Data RAM must remain demand-zero. I-cache maintenance can populate its
    // host backing before the guest touches it; publish only actual code pages.
    (void)h;
    return (int)hv_vm_map(memory,gpa,size,HV_MEMORY_READ|HV_MEMORY_WRITE|HV_MEMORY_EXEC);
}
int trames_hvf_unmap(struct host *h, uint64_t gpa, size_t size) {
    (void)h; return (int)hv_vm_unmap(gpa,size);
}
void trames_hvf_icache(void *memory, size_t size) { sys_icache_invalidate(memory,size); }
void trames_hvf_destroy(struct host *h) {
    if (!h) return;
    if (h->cpu_created) hv_vcpu_destroy(h->cpu);
    if (h->vectors) { hv_vm_unmap(VECTOR_GPA,h->granule); munmap(h->vectors,h->granule); }
    while (h->tables) {
        struct table *t=h->tables; h->tables=t->next;
        hv_vm_unmap(t->gpa,h->granule); munmap(t->memory,h->granule); free(t);
    }
    hv_vm_destroy(); free(h);
}
struct host *trames_hvf_create(int *error) {
    *error=(int)hv_vm_create(NULL); if (*error) return NULL;
    struct host *h=calloc(1,sizeof(*h));
    if (!h) { hv_vm_destroy(); *error=-1; return NULL; }
    h->granule=(size_t)sysconf(_SC_PAGESIZE); h->next_table=TABLE_GPA;
    h->root=new_table(h,&h->root_gpa);
    h->vectors=anonymous(h->granule);
    if (!h->root || !h->vectors) goto fail;
    memcpy(h->vectors,trames_hvf_vectors_begin,(size_t)(trames_hvf_vectors_end-trames_hvf_vectors_begin));
    memcpy((char *)h->vectors+0x800,trames_hvf_tlbi_begin,(size_t)(trames_hvf_tlbi_end-trames_hvf_tlbi_begin));
    memcpy((char *)h->vectors+0x900,trames_hvf_resume_begin,(size_t)(trames_hvf_resume_end-trames_hvf_resume_begin));
    sys_icache_invalidate(h->vectors,h->granule);
    *error=(int)hv_vm_map(h->vectors,VECTOR_GPA,h->granule,HV_MEMORY_READ|HV_MEMORY_EXEC);
    if (*error) goto fail;
    for (size_t offset=0; offset<h->granule; offset+=PAGE)
        if (trames_hvf_page(h,VECTOR_VA+offset,VECTOR_GPA+offset,1|4|8)) goto fail;
    *error=(int)hv_vcpu_create(&h->cpu,&h->exit,NULL); if (*error) goto fail;
    h->cpu_created=true;
    if ((*error=(int)hv_vcpu_set_sys_reg(h->cpu,HV_SYS_REG_TTBR0_EL1,h->root_gpa)) ||
        (*error=(int)hv_vcpu_set_sys_reg(h->cpu,HV_SYS_REG_TCR_EL1,16|(1<<8)|(1<<10)|(3<<12)|(1<<23)|(UINT64_C(2)<<32)|(UINT64_C(1)<<37))) ||
        (*error=(int)hv_vcpu_set_sys_reg(h->cpu,HV_SYS_REG_MAIR_EL1,0xff)) ||
        (*error=(int)hv_vcpu_set_sys_reg(h->cpu,HV_SYS_REG_VBAR_EL1,VECTOR_VA)) ||
        (*error=(int)hv_vcpu_set_sys_reg(h->cpu,HV_SYS_REG_CPACR_EL1,3<<20)) ||
        (*error=(int)hv_vcpu_set_sys_reg(h->cpu,HV_SYS_REG_SCTLR_EL1,UINT64_C(0x30d00800)|1|4|(1<<12)))) goto fail;
    *error=0; return h;
fail:
    if (!*error) *error=-1;
    trames_hvf_destroy(h); return NULL;
}
int trames_hvf_run(struct host *h, struct state *s, uint64_t nanoseconds, struct result *out) {
    hv_return_t e;
    if (h->tlb_dirty) {
        if ((e=hv_vcpu_set_vtimer_mask(h->cpu,true)) ||
            (e=hv_vcpu_set_reg(h->cpu,HV_REG_CPSR,0x3c5)) ||
            (e=hv_vcpu_set_reg(h->cpu,HV_REG_PC,VECTOR_VA+0x800)) ||
            (e=hv_vcpu_run(h->cpu))) return (int)e;
        if (h->exit->reason!=HV_EXIT_REASON_EXCEPTION || h->exit->exception.syndrome>>26!=0x16) return -2;
        h->tlb_dirty=false;
    }
    for (unsigned i=0;i<31;i++) if ((e=hv_vcpu_set_reg(h->cpu,(hv_reg_t)(HV_REG_X0+i),s->x[i]))) return (int)e;
    for (unsigned i=0;i<32;i++) {
        union { __uint128_t bits; hv_simd_fp_uchar16_t vector; } value={.bits=s->vectors[i]};
        if ((e=hv_vcpu_set_simd_fp_reg(h->cpu,(hv_simd_fp_reg_t)(HV_SIMD_FP_REG_Q0+i),value.vector))) return (int)e;
    }
    if ((e=hv_vcpu_set_sys_reg(h->cpu,HV_SYS_REG_SP_EL0,s->sp)) ||
        (e=hv_vcpu_set_sys_reg(h->cpu,HV_SYS_REG_TPIDR_EL0,s->tpidr)) ||
        (e=hv_vcpu_set_reg(h->cpu,HV_REG_FPCR,s->fpcr)) ||
        (e=hv_vcpu_set_reg(h->cpu,HV_REG_FPSR,s->fpsr)) ||
        // Each task/slice may invalidate an exclusive reservation. Enter through
        // two EL1 instructions so reservations never leak between guest tasks.
        (e=hv_vcpu_set_sys_reg(h->cpu,HV_SYS_REG_ELR_EL1,s->pc)) ||
        (e=hv_vcpu_set_sys_reg(h->cpu,HV_SYS_REG_SPSR_EL1,0x3c0|(s->nzcv&0xf0000000))) ||
        (e=hv_vcpu_set_reg(h->cpu,HV_REG_CPSR,0x3c5)) ||
        (e=hv_vcpu_set_reg(h->cpu,HV_REG_PC,VECTOR_VA+0x900)) ||
        (e=hv_vcpu_set_sys_reg(h->cpu,HV_SYS_REG_CNTV_CVAL_EL0,ticks()+nanoseconds*frequency()/UINT64_C(1000000000))) ||
        (e=hv_vcpu_set_sys_reg(h->cpu,HV_SYS_REG_CNTV_CTL_EL0,1)) ||
        (e=hv_vcpu_set_vtimer_mask(h->cpu,false)) ||
        (e=hv_vcpu_run(h->cpu))) return (int)e;
    for (unsigned i=0;i<31;i++) if ((e=hv_vcpu_get_reg(h->cpu,(hv_reg_t)(HV_REG_X0+i),&s->x[i]))) return (int)e;
    for (unsigned i=0;i<32;i++) {
        union { __uint128_t bits; hv_simd_fp_uchar16_t vector; } value;
        if ((e=hv_vcpu_get_simd_fp_reg(h->cpu,(hv_simd_fp_reg_t)(HV_SIMD_FP_REG_Q0+i),&value.vector))) return (int)e;
        s->vectors[i]=value.bits;
    }
    uint64_t n;
    if ((e=hv_vcpu_get_sys_reg(h->cpu,HV_SYS_REG_SP_EL0,&s->sp)) ||
        (e=hv_vcpu_get_sys_reg(h->cpu,HV_SYS_REG_TPIDR_EL0,&s->tpidr)) ||
        (e=hv_vcpu_get_reg(h->cpu,HV_REG_FPCR,&n))) return (int)e;
    s->fpcr=(uint32_t)n;
    if ((e=hv_vcpu_get_reg(h->cpu,HV_REG_FPSR,&n))) return (int)e; s->fpsr=(uint32_t)n;
    out->kind=0; out->syndrome=h->exit->exception.syndrome; out->address=0;
    uint64_t raw_pc, raw_pstate;
    if ((e=hv_vcpu_get_reg(h->cpu,HV_REG_PC,&raw_pc)) ||
        (e=hv_vcpu_get_reg(h->cpu,HV_REG_CPSR,&raw_pstate))) return (int)e;
    // A timer can exit after EL0 took SVC/abort, but before the EL1 HVC stub
    // executes. Preserve that saved EL0 exception; never resume an EL1 vector
    // PC as EL0 merely because this exit was reported as a timer/cancellation.
    bool saved_exception=(h->exit->reason==HV_EXIT_REASON_EXCEPTION && h->exit->exception.syndrome>>26==0x16) ||
        ((raw_pstate&15)==5 && raw_pc>=VECTOR_VA && raw_pc<VECTOR_VA+0x800);
    if (saved_exception) {
        if ((e=hv_vcpu_get_sys_reg(h->cpu,HV_SYS_REG_ESR_EL1,&out->syndrome)) ||
            (e=hv_vcpu_get_sys_reg(h->cpu,HV_SYS_REG_FAR_EL1,&out->address)) ||
            (e=hv_vcpu_get_sys_reg(h->cpu,HV_SYS_REG_ELR_EL1,&s->pc)) ||
            (e=hv_vcpu_get_sys_reg(h->cpu,HV_SYS_REG_SPSR_EL1,&n))) return (int)e;
        unsigned ec=(unsigned)(out->syndrome>>26);
        out->kind=ec==0x15 ? 1 : (ec==0x20 || ec==0x24) ? 2 : 3;
    } else if ((raw_pstate&15)==5 && raw_pc>=VECTOR_VA+0x900 && raw_pc<VECTOR_VA+0x908) {
        // Timer/cancellation before ERET: retain the saved EL0 continuation.
        if ((e=hv_vcpu_get_sys_reg(h->cpu,HV_SYS_REG_ELR_EL1,&s->pc)) ||
            (e=hv_vcpu_get_sys_reg(h->cpu,HV_SYS_REG_SPSR_EL1,&n))) return (int)e;
    } else {
        s->pc=raw_pc; n=raw_pstate;
        if (h->exit->reason!=HV_EXIT_REASON_VTIMER_ACTIVATED && h->exit->reason!=HV_EXIT_REASON_CANCELED) out->kind=3;
    }
    s->nzcv=(uint32_t)n&0xf0000000; out->pc=s->pc;
    return 0;
}
