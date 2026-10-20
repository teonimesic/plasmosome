#ifndef _POSIX_C_SOURCE
#define _POSIX_C_SOURCE 200809L
#endif

#include <errno.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>
#include <unistd.h>

int sandbox_init(const char *profile, uint64_t flags, char **errorbuf);

int main(int argc, char **argv) {
    if (argc != 2) {
        return 64;
    }
    if (strcmp(argv[1], "none") != 0) {
        char *error = NULL;
        if (sandbox_init(argv[1], 0, &error) != 0) {
            fprintf(stderr, "sandbox_init refused the profile: %s\n", error != NULL ? error : "no message");
            return 71;
        }
    }
    if (write(STDOUT_FILENO, "ready\n", 6) != 6) {
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
