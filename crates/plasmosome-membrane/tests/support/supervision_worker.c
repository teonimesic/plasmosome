#include <sys/types.h>
#include <sys/stat.h>
#include <fcntl.h>
#include <signal.h>
#include <errno.h>
#include <stdbool.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

static int descriptor(const char *text) {
    char *end = NULL;
    long value = strtol(text, &end, 10);
    if (end == text || *end != '\0' || value < 0 || value > 0x7fffffffL) {
        return -1;
    }
    return (int)value;
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

static void named_worker(const char *liveness_path, const char *control_path) {
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
    if (argc != 5) {
        return 64;
    }
    bool named = strcmp(argv[1], "named-exit") == 0;
    int ready = named ? -1 : descriptor(argv[2]);
    int control = named ? -1 : descriptor(argv[3]);
    if (!named && (ready < 0 || control < 0)) {
        return 65;
    }

    pid_t worker = fork();
    if (worker < 0) {
        return 66;
    }
    if (worker == 0) {
        if (named) {
            named_worker(argv[2], argv[3]);
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
