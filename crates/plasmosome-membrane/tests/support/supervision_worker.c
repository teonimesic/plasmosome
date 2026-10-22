#ifndef _POSIX_C_SOURCE
#define _POSIX_C_SOURCE 200809L
#endif

#include <sys/types.h>
#include <sys/stat.h>
#include <fcntl.h>
#include <signal.h>
#include <errno.h>
#include <stdbool.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <time.h>

static int descriptor(const char *text) {
    char *end = NULL;
    long value = strtol(text, &end, 10);
    if (end == text || *end != '\0' || value < 0 || value > 0x7fffffffL) {
        return -1;
    }
    return (int)value;
}

static bool is_open(int fd) {
    return fcntl(fd, F_GETFD) != -1;
}

static void check_pipe_ends(int ready, int control, int parent_ready, int parent_control) {
    if (!is_open(ready)) {
        _exit(75);
    }
    if (!is_open(control) || is_open(parent_ready) || is_open(parent_control)) {
        char leaked = 'L';
        if (write(ready, &leaked, 1) != 1) {
            _exit(76);
        }
        _exit(75);
    }
}

static void descriptor_worker(int ready, int control, const char *liveness_path) {
    int liveness = open(liveness_path, O_WRONLY);
    if (liveness < 0) {
        _exit(67);
    }
    char byte = 'x';
    if (write(ready, &byte, 1) != 1) {
        _exit(68);
    }
    close(ready);
    ssize_t read_result;
    do {
        read_result = read(control, &byte, 1);
    } while (read_result < 0 && errno == EINTR);
    close(control);
    close(liveness);
    _exit(read_result >= 0 ? 0 : 69);
}

static void wait_until_orphaned(pid_t leader) {
    struct timespec pause = {0, 1000000};
    while (getppid() == leader) {
        nanosleep(&pause, NULL);
    }
}

static void named_worker(pid_t leader, const char *liveness_path, const char *control_path) {
    wait_until_orphaned(leader);
    int liveness = open(liveness_path, O_WRONLY);
    if (liveness < 0) {
        _exit(70);
    }
    char byte = 'x';
    if (write(liveness, &byte, 1) != 1) {
        _exit(72);
    }
    int control = open(control_path, O_RDONLY);
    if (control < 0) {
        _exit(73);
    }
    ssize_t read_result;
    do {
        read_result = read(control, &byte, 1);
    } while (read_result < 0 && errno == EINTR);
    close(control);
    close(liveness);
    _exit(read_result >= 0 ? 0 : 74);
}

int main(int argc, char **argv) {
    if (argc < 2) {
        return 64;
    }
    bool late = strcmp(argv[1], "named-exit-late") == 0;
    bool named = late || strcmp(argv[1], "named-exit") == 0;
    if (argc != (named ? 5 : 7)) {
        return 64;
    }
    int ready = named ? -1 : descriptor(argv[2]);
    int control = named ? -1 : descriptor(argv[3]);
    if (!named) {
        int parent_ready = descriptor(argv[5]);
        int parent_control = descriptor(argv[6]);
        if (ready < 0 || control < 0 || parent_ready < 0 || parent_control < 0) {
            return 65;
        }
        check_pipe_ends(ready, control, parent_ready, parent_control);
    }

    pid_t leader = getpid();
    pid_t worker = fork();
    if (worker < 0) {
        return 66;
    }
    if (worker == 0) {
        if (late) {
            wait_until_orphaned(leader);
        }
        if (named) {
            named_worker(leader, argv[2], argv[3]);
        }
        descriptor_worker(ready, control, argv[4]);
    }

    if (!named) {
        close(ready);
        close(control);
    }
    if (named || strcmp(argv[1], "exit") == 0) {
        return 7;
    }
    for (;;) {
        pause();
    }
}
