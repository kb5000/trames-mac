// SPDX-License-Identifier: GPL-3.0-or-later
// Small EL1 exception/page-table adapter. Linux remains in the host runtime.
#include <Hypervisor/Hypervisor.h>
#include <dispatch/dispatch.h>
#include <stdatomic.h>
#include <libkern/OSCacheControl.h>
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
#include <stdlib.h>
#include <pthread.h>
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
struct free_gpa { uint64_t value; struct free_gpa *next; };
struct machine {
    size_t granule;
    void *vectors;
    pthread_mutex_t addresses;
    uint64_t next_gpa;
    bool asids[256];
    struct free_gpa *free_gpas;
};
struct address_space {
    struct table *tables;
    uint64_t *root, root_gpa;
    struct address_space *next;
};
struct host {
    struct machine *machine;
    bool owns_machine;
    unsigned asid;
    hv_vcpu_t cpu;
    hv_vcpu_exit_t *exit;
    bool cpu_created, tlb_dirty;
    size_t granule;
    struct address_space *space, *spaces;
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
uint64_t trames_hvf_reserve(struct machine *m) {
    pthread_mutex_lock(&m->addresses);
    struct free_gpa *entry=m->free_gpas;
    uint64_t gpa;
    if (entry) { gpa=entry->value; m->free_gpas=entry->next; free(entry); }
    else { gpa=m->next_gpa; m->next_gpa+=m->granule; }
    pthread_mutex_unlock(&m->addresses);
    return gpa;
}
void trames_hvf_release(struct machine *m, uint64_t gpa) {
    struct free_gpa *entry=malloc(sizeof(*entry));
    if (!entry) return; // A lost address slot cannot invalidate live RAM.
    entry->value=gpa;
    pthread_mutex_lock(&m->addresses);
    entry->next=m->free_gpas; m->free_gpas=entry;
    pthread_mutex_unlock(&m->addresses);
}
static uint64_t *table_for(struct host *h, uint64_t gpa) {
    for (struct table *t=h->space->tables; t; t=t->next)
        if (gpa >= t->gpa && gpa < t->gpa+h->granule)
            return (uint64_t *)((char *)t->memory + gpa-t->gpa);
    return NULL;
}
static uint64_t *new_table(struct host *h, uint64_t *gpa) {
    struct table *t=calloc(1,sizeof(*t));
    if (!t) return NULL;
    t->memory=anonymous(h->granule);
    if (!t->memory) { free(t); return NULL; }
    t->gpa=trames_hvf_reserve(h->machine);
    if (hv_vm_map(t->memory,t->gpa,h->granule,HV_MEMORY_READ|HV_MEMORY_WRITE)) {
        trames_hvf_release(h->machine,t->gpa);
        munmap(t->memory,h->granule); free(t); return NULL;
    }
    t->next=h->space->tables; h->space->tables=t; *gpa=t->gpa;
    return t->memory;
}
static uint64_t *leaf(struct host *h, uint64_t va, bool create) {
    uint64_t *table=h->space->root;
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
    uint64_t *chain[4]={h->space->root}, gpas[4]={h->space->root_gpa}; unsigned indexes[3];
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
        struct table **link=&h->space->tables;
        while (*link && (*link)->gpa!=gpas[level]) link=&(*link)->next;
        if (!*link) return -1;
        struct table *t=*link;
        hv_return_t e=hv_vm_unmap(t->gpa,h->granule); if (e) return (int)e;
        *link=t->next;trames_hvf_release(h->machine,t->gpa);munmap(t->memory,h->granule);free(t);
    }
    return 0;
}
int trames_hvf_page(struct host *h, uint64_t va, uint64_t gpa, unsigned flags) {
    if (!flags) return remove_page(h,va);
    uint64_t *p=leaf(h,va,flags!=0);
    if (!p) return flags ? -1 : 0;
    // flags: read=1, write=2, execute=4, EL1-only=8.
    uint64_t ap=(flags&8) ? 0 : (flags&2) ? 1 : 3;
    uint64_t descriptor=(gpa&ADDRESS_MASK)|3|(UINT64_C(1)<<11)|(UINT64_C(1)<<10)|(UINT64_C(3)<<8)|(ap<<6);
    if (!(flags&8)) descriptor|=UINT64_C(1)<<53;
    if (!(flags&4) || (flags&8)) descriptor|=UINT64_C(1)<<54;
    if ((flags&8) && (flags&4)) descriptor&=~(UINT64_C(1)<<53);
    if (*p!=descriptor) { *p=descriptor; h->tlb_dirty=true; }
    return 0;
}

// Address-space tables persist independently of the worker's current task.
// One ASID remains allocated per vCPU. Switching roots always flushes that ASID
// through the existing EL1 trampoline before EL0 resumes, including ASID reuse.
static int free_tables(struct host *h, struct address_space *space) {
    while (space->tables) {
        struct table *t=space->tables;
        hv_return_t error=hv_vm_unmap(t->gpa,h->granule);
        if (error) return (int)error;
        space->tables=t->next; trames_hvf_release(h->machine,t->gpa);
        munmap(t->memory,h->granule); free(t);
    }
    return 0;
}
struct address_space *trames_hvf_space_create(struct host *h, int *error) {
    struct address_space *space=calloc(1,sizeof(*space));
    if (!space) { *error=-1; return NULL; }
    struct address_space *previous=h->space;
    h->space=space;
    space->root=new_table(h,&space->root_gpa);
    *error=space->root ? 0 : -1;
    for (size_t offset=0; !*error && offset<h->granule; offset+=PAGE)
        *error=trames_hvf_page(h,VECTOR_VA+offset,VECTOR_GPA+offset,1|4|8);
    h->space=previous;
    if (*error) { free_tables(h,space); free(space); return NULL; }
    space->next=h->spaces; h->spaces=space;
    return space;
}
struct address_space *trames_hvf_space_current(struct host *h) { return h->space; }
int trames_hvf_space_select(struct host *h, struct address_space *space) {
    if (h->space==space) return 0;
    hv_return_t error=hv_vcpu_set_sys_reg(h->cpu,HV_SYS_REG_TTBR0_EL1,space->root_gpa|((uint64_t)h->asid<<48));
    if (error) return (int)error;
    h->space=space; h->tlb_dirty=true;
    return 0;
}
int trames_hvf_space_destroy(struct host *h, struct address_space *space) {
    if (space==h->space) return -1;
    struct address_space **link=&h->spaces;
    while (*link && *link!=space) link=&(*link)->next;
    if (!*link) return -1;
    int error=free_tables(h,space); if (error) return error;
    *link=space->next; free(space); return 0;
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
int trames_hvf_protect(uint64_t gpa, size_t size, unsigned flags) {
    return (int)hv_vm_protect(gpa,size,flags);
}
void trames_hvf_vm_destroy(struct machine *m) {
    if (!m) return;
    if (m->vectors) { hv_vm_unmap(VECTOR_GPA,m->granule); munmap(m->vectors,m->granule); }
    hv_vm_destroy();
    while (m->free_gpas) { struct free_gpa *p=m->free_gpas; m->free_gpas=p->next; free(p); }
    pthread_mutex_destroy(&m->addresses); free(m);
}
struct machine *trames_hvf_vm_create(int *error) {
    *error=(int)hv_vm_create(NULL); if (*error) return NULL;
    struct machine *m=calloc(1,sizeof(*m));
    if (!m) { hv_vm_destroy(); *error=-1; return NULL; }
    pthread_mutex_init(&m->addresses,NULL);
    m->granule=(size_t)sysconf(_SC_PAGESIZE); m->next_gpa=TABLE_GPA;
    m->vectors=anonymous(m->granule);
    if (!m->vectors) { *error=-1; goto fail; }
    memcpy(m->vectors,trames_hvf_vectors_begin,(size_t)(trames_hvf_vectors_end-trames_hvf_vectors_begin));
    memcpy((char *)m->vectors+0x800,trames_hvf_tlbi_begin,(size_t)(trames_hvf_tlbi_end-trames_hvf_tlbi_begin));
    memcpy((char *)m->vectors+0x900,trames_hvf_resume_begin,(size_t)(trames_hvf_resume_end-trames_hvf_resume_begin));
    sys_icache_invalidate(m->vectors,m->granule);
    *error=(int)hv_vm_map(m->vectors,VECTOR_GPA,m->granule,HV_MEMORY_READ|HV_MEMORY_EXEC);
    if (*error) goto fail;
    return m;
fail:
    trames_hvf_vm_destroy(m); return NULL;
}
void trames_hvf_cpu_destroy(struct host *h) {
    if (!h) return;
    if (h->cpu_created) hv_vcpu_destroy(h->cpu);
    while (h->spaces) {
        struct address_space *space=h->spaces; h->spaces=space->next;
        free_tables(h,space); free(space);
    }
    pthread_mutex_lock(&h->machine->addresses); h->machine->asids[h->asid]=false; pthread_mutex_unlock(&h->machine->addresses);
    free(h);
}
struct host *trames_hvf_cpu_create(struct machine *m, int *error) {
    struct host *h=calloc(1,sizeof(*h));
    if (!h) { *error=-1; return NULL; }
    h->machine=m; h->granule=m->granule;
    pthread_mutex_lock(&m->addresses);
    for (unsigned i=1;i<256;i++) if (!m->asids[i]) { m->asids[i]=true; h->asid=i; break; }
    pthread_mutex_unlock(&m->addresses);
    if (!h->asid) { *error=-1; goto fail; }
    h->space=trames_hvf_space_create(h,error);
    if (!h->space) goto fail;
    *error=(int)hv_vcpu_create(&h->cpu,&h->exit,NULL); if (*error) goto fail;
    h->cpu_created=true;
    if ((*error=(int)hv_vcpu_set_sys_reg(h->cpu,HV_SYS_REG_TTBR0_EL1,h->space->root_gpa|((uint64_t)h->asid<<48))) ||
        (*error=(int)hv_vcpu_set_sys_reg(h->cpu,HV_SYS_REG_TCR_EL1,16|(1<<8)|(1<<10)|(3<<12)|(1<<23)|(UINT64_C(2)<<32)|(UINT64_C(1)<<37))) ||
        (*error=(int)hv_vcpu_set_sys_reg(h->cpu,HV_SYS_REG_MAIR_EL1,0xff)) ||
        (*error=(int)hv_vcpu_set_sys_reg(h->cpu,HV_SYS_REG_VBAR_EL1,VECTOR_VA)) ||
        (*error=(int)hv_vcpu_set_sys_reg(h->cpu,HV_SYS_REG_CPACR_EL1,3<<20)) ||
        // Match Linux: EL0 may read CNTVCT/CNTFRQ, but cannot control timers.
        (*error=(int)hv_vcpu_set_sys_reg(h->cpu,HV_SYS_REG_CNTKCTL_EL1,1<<1)) ||
        (*error=(int)hv_vcpu_set_sys_reg(h->cpu,HV_SYS_REG_SCTLR_EL1,UINT64_C(0x30d00800)|1|4|(1<<12)|(1<<15)|(1<<26)))) goto fail;
    *error=0; return h;
fail:
    if (!*error) *error=-1;
    trames_hvf_cpu_destroy(h); return NULL;
}
struct host *trames_hvf_create(int *error) {
    struct machine *m=trames_hvf_vm_create(error); if (!m) return NULL;
    struct host *h=trames_hvf_cpu_create(m,error);
    if (!h) { trames_hvf_vm_destroy(m); return NULL; }
    h->owns_machine=true; return h;
}
void trames_hvf_destroy(struct host *h) {
    if (!h) return;
    struct machine *m=h->machine; bool owned=h->owns_machine;
    trames_hvf_cpu_destroy(h);
    if (owned) trames_hvf_vm_destroy(m);
}
int trames_hvf_run(struct host *h, struct state *s, uint64_t nanoseconds, struct result *out) {
    hv_return_t e;
    if (h->tlb_dirty) {
        if ((e=hv_vcpu_set_reg(h->cpu,HV_REG_X0,(uint64_t)h->asid<<48)) ||
            (e=hv_vcpu_set_vtimer_mask(h->cpu,true)) ||
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
        if (h->exit->reason!=HV_EXIT_REASON_VTIMER_ACTIVATED && h->exit->reason!=HV_EXIT_REASON_CANCELED) {
            unsigned ec=(unsigned)(out->syndrome>>26);
            out->kind=(ec==0x20 || ec==0x24) ? 4 : 3;
            out->address=h->exit->exception.physical_address;
        }
    }
    s->nzcv=(uint32_t)n&0xf0000000; out->pc=s->pc;
    return 0;
}

uint32_t trames_hvf_max_vcpus(void) {
    uint32_t count=1;
    if (hv_vm_get_max_vcpu_count(&count)) return 1;
    return count;
}

// The legacy EVFILT_VM filter is unsupported on current macOS. libdispatch's
// memory-pressure source uses the supported OS notification mechanism. Its
// shared utility queue only latches metadata; the session thread does eviction.
static dispatch_source_t pressure_source;
static pthread_once_t pressure_once=PTHREAD_ONCE_INIT;
static atomic_uint pressure_bits;
static void pressure_event(void *context) {
    unsigned bits=(unsigned)dispatch_source_get_data((dispatch_source_t)context);
    atomic_store_explicit(&pressure_bits,bits,memory_order_relaxed);
}
static void pressure_init(void) {
    pressure_source=dispatch_source_create(DISPATCH_SOURCE_TYPE_MEMORYPRESSURE,0,
        DISPATCH_MEMORYPRESSURE_NORMAL|DISPATCH_MEMORYPRESSURE_WARN|DISPATCH_MEMORYPRESSURE_CRITICAL,
        dispatch_get_global_queue(QOS_CLASS_UTILITY,0));
    if (!pressure_source) return;
    dispatch_set_context(pressure_source,pressure_source);
    dispatch_source_set_event_handler_f(pressure_source,pressure_event);
    dispatch_resume(pressure_source);
}
int trames_hvf_pressure_supported(void) {
    pthread_once(&pressure_once,pressure_init); return pressure_source!=NULL;
}
unsigned trames_hvf_take_pressure(void) {
    return atomic_exchange_explicit(&pressure_bits,0,memory_order_relaxed);
}
// Referenced only by signed unit tests; no product CLI or IPC exposes injection.
void trames_hvf_inject_pressure_for_test(unsigned bits) {
    atomic_store_explicit(&pressure_bits,bits,memory_order_relaxed);
}
