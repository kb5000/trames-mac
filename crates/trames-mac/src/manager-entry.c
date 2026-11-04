// SPDX-License-Identifier: GPL-3.0-or-later
// Finder has an explicit GUI entry; CLI stdin is never used to infer GUI intent.
#include <mach-o/dyld.h>
#include <libgen.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
int main(void) {
    uint32_t size = 0;
    _NSGetExecutablePath(NULL, &size);
    char *path = malloc((size_t)size + 32);
    if (!path || _NSGetExecutablePath(path, &size)) return 1;
    char *last = strrchr(path, '/');
    if (!last) { free(path); return 1; }
    strcpy(last + 1, "trames-mac-run");
    char *argv[] = {path, "gui", NULL};
    execv(path, argv);
    perror("Trames manager");
    free(path);
    return 1;
}
