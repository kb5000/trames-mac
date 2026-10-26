// SPDX-License-Identifier: GPL-3.0-or-later
// Identical static AArch64 ELF runs on native Linux and the HVF process kernel.
#include <assert.h>
#include <errno.h>
#include <pthread.h>
#include <signal.h>
#include <stdatomic.h>
#include <stdio.h>
#include <stdlib.h>
#include <sys/mman.h>
#include <sys/wait.h>
#include <unistd.h>

static _Thread_local unsigned tls;
static atomic_uint count;
static pthread_mutex_t mutex = PTHREAD_MUTEX_INITIALIZER;
static pthread_cond_t condition = PTHREAD_COND_INITIALIZER;
static unsigned ready;
static void *thread(void *argument) {
    unsigned id = (unsigned)(size_t)argument;
    tls = id;
    assert(pthread_mutex_lock(&mutex) == 0);
    ready++;
    assert(pthread_cond_broadcast(&condition) == 0);
    while (ready != 4) assert(pthread_cond_wait(&condition, &mutex) == 0);
    assert(pthread_mutex_unlock(&mutex) == 0);
    for (unsigned i = 0; i < 20000; i++) {
        atomic_fetch_add_explicit(&count, 1, memory_order_relaxed);
        assert(tls == id);
        if (i % 1000 == 0) sched_yield();
    }
    return (void *)(size_t)tls;
}

int main(void) {
    unsigned char *memory = mmap(0, 256 * 4096, PROT_READ | PROT_WRITE,
                                 MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    assert(memory != MAP_FAILED);
    for (unsigned i = 0; i < 256; i++) memory[i * 4096] = i;
    int pipefd[2];
    assert(pipe(pipefd) == 0);
    pid_t child = fork();
    assert(child >= 0);
    if (!child) {
        close(pipefd[0]);
        for (unsigned i = 0; i < 256; i++) {
            assert(memory[i * 4096] == (unsigned char)i);
            memory[i * 4096] = 255 - i;
        }
        assert(write(pipefd[1], "child", 5) == 5);
        close(pipefd[1]);
        _exit(23);
    }
    close(pipefd[1]);
    char bytes[6] = {0};
    assert(read(pipefd[0], bytes, 5) == 5);
    assert(read(pipefd[0], bytes + 5, 1) == 0);
    close(pipefd[0]);
    int status;
    assert(waitpid(child, &status, 0) == child);
    assert(WIFEXITED(status) && WEXITSTATUS(status) == 23);
    for (unsigned i = 0; i < 256; i++) assert(memory[i * 4096] == (unsigned char)i);
    assert(munmap(memory, 256 * 4096) == 0);
    puts("fork-cow-pipe-wait: ok");
    pthread_t threads[4];
    for (unsigned i = 0; i < 4; i++) assert(pthread_create(&threads[i], 0, thread, (void *)(size_t)(i + 1)) == 0);
    for (unsigned i = 0; i < 4; i++) {
        void *value;
        assert(pthread_join(threads[i], &value) == 0);
        assert((unsigned)(size_t)value == i + 1);
    }
    assert(atomic_load(&count) == 80000 && tls == 0);
    puts("threads-tls-futex-atomics: ok");
    child = fork();
    assert(child >= 0);
    if (!child) { raise(SIGSTOP); _exit(31); }
    assert(waitpid(child, &status, WUNTRACED) == child && WIFSTOPPED(status));
    assert(kill(child, SIGCONT) == 0);
    assert(waitpid(child, &status, 0) == child && WIFEXITED(status) && WEXITSTATUS(status) == 31);
    puts("stop-continue-wait: ok");
    return 0;
}
