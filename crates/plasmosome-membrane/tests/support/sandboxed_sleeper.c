#ifndef _POSIX_C_SOURCE
#define _POSIX_C_SOURCE 200809L
#endif

#include <errno.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>
#include <unistd.h>

#define UNIQUE_IDENTIFIERS_FLAVOR 17

struct unique_identifiers {
    uint8_t executable_uuid[16];
    uint64_t unique_id;
    uint64_t parent_unique_id;
    int32_t id_version;
    int32_t original_parent_id_version;
    uint64_t reserved[2];
};

int sandbox_init(const char *profile, uint64_t flags, char **errorbuf);
int proc_pidinfo(int pid, int flavor, uint64_t arg, void *buffer, int buffersize);

int main(int argc, char **argv) {
    if (argc != 2) {
        return 64;
    }
    struct unique_identifiers identifiers;
    int written = proc_pidinfo(getpid(), UNIQUE_IDENTIFIERS_FLAVOR, 0, &identifiers, (int)sizeof identifiers);
    if (written != (int)sizeof identifiers) {
        fprintf(stderr, "proc_pidinfo read %d bytes of this process's unique identifiers: %s\n", written, strerror(errno));
        return 70;
    }
    if (strcmp(argv[1], "none") != 0) {
        char *error = NULL;
        if (sandbox_init(argv[1], 0, &error) != 0) {
            fprintf(stderr, "sandbox_init refused the profile: %s\n", error != NULL ? error : "no message");
            return 71;
        }
    }
    if (printf("ready %llu\n", (unsigned long long)identifiers.unique_id) < 0 || fflush(stdout) != 0) {
        return 74;
    }
    char byte;
    for (;;) {
        ssize_t got = read(STDIN_FILENO, &byte, 1);
        if (got == 0) {
            return 0;
        }
        if (got < 0 && errno != EINTR) {
            return 75;
        }
    }
}
