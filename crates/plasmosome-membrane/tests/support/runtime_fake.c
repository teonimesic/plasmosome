#if defined(__APPLE__)
#define _DARWIN_C_SOURCE
#else
#define _GNU_SOURCE
#endif

#include <sys/types.h>
#include <sys/stat.h>
#include <dirent.h>
#include <errno.h>
#include <fcntl.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#if defined(__APPLE__)
#include <libproc.h>
#endif

#define USAGE 64
#define BROKEN 65
#define NO_EOF 99
#define RECIPE_FD 3

extern char **environ;

static void on_alarm(int signal_number) {
    (void)signal_number;
    _exit(NO_EOF);
}

static int ascending(const void *left, const void *right) {
    int a = *(const int *)left;
    int b = *(const int *)right;
    return (a > b) - (a < b);
}

static int *open_descriptors(size_t *count) {
    size_t found = 0;
    int *numbers = NULL;
#if defined(__APPLE__)
    int bytes = proc_pidinfo(getpid(), PROC_PIDLISTFDS, 0, NULL, 0);
    if (bytes <= 0) {
        _exit(BROKEN);
    }
    struct proc_fdinfo *table = malloc((size_t)bytes);
    if (table == NULL) {
        _exit(BROKEN);
    }
    bytes = proc_pidinfo(getpid(), PROC_PIDLISTFDS, 0, table, bytes);
    if (bytes < 0) {
        _exit(BROKEN);
    }
    found = (size_t)bytes / sizeof(struct proc_fdinfo);
    numbers = malloc(sizeof(int) * (found + 1));
    if (numbers == NULL) {
        _exit(BROKEN);
    }
    for (size_t index = 0; index < found; index++) {
        numbers[index] = table[index].proc_fd;
    }
    free(table);
#else
    DIR *directory = opendir("/proc/self/fd");
    if (directory == NULL) {
        _exit(BROKEN);
    }
    int own = dirfd(directory);
    size_t capacity = 16;
    numbers = malloc(sizeof(int) * capacity);
    if (numbers == NULL) {
        _exit(BROKEN);
    }
    struct dirent *entry;
    while ((entry = readdir(directory)) != NULL) {
        if (entry->d_name[0] == '.') {
            continue;
        }
        int number = atoi(entry->d_name);
        if (number == own) {
            continue;
        }
        if (found == capacity) {
            capacity *= 2;
            numbers = realloc(numbers, sizeof(int) * capacity);
            if (numbers == NULL) {
                _exit(BROKEN);
            }
        }
        numbers[found++] = number;
    }
    closedir(directory);
#endif
    qsort(numbers, found, sizeof(int), ascending);
    *count = found;
    return numbers;
}

static const char *kind(mode_t mode) {
    if (S_ISCHR(mode)) {
        return "chr";
    }
    if (S_ISFIFO(mode)) {
        return "fifo";
    }
    if (S_ISSOCK(mode)) {
        return "sock";
    }
    if (S_ISREG(mode)) {
        return "reg";
    }
    if (S_ISDIR(mode)) {
        return "dir";
    }
    return "other";
}

static const char *access_mode(int flags) {
    if (flags == -1) {
        return "unknown";
    }
    switch (flags & O_ACCMODE) {
    case O_RDONLY:
        return "r";
    case O_WRONLY:
        return "w";
    case O_RDWR:
        return "rw";
    default:
        return "other";
    }
}

static void report_descriptors(FILE *report) {
    struct stat null_device;
    if (stat("/dev/null", &null_device) != 0) {
        _exit(BROKEN);
    }
    size_t count = 0;
    int *numbers = open_descriptors(&count);
    for (size_t index = 0; index < count; index++) {
        int number = numbers[index];
        struct stat status;
        if (fstat(number, &status) != 0) {
            fprintf(report, "fd=%d kind=unreadable null=0 nonblock=0 access=unknown\n", number);
            continue;
        }
        int flags = fcntl(number, F_GETFL);
        int is_null = S_ISCHR(status.st_mode) && status.st_rdev == null_device.st_rdev;
        int nonblock = flags != -1 && (flags & O_NONBLOCK) != 0;
        fprintf(report, "fd=%d kind=%s null=%d nonblock=%d access=%s\n", number,
                kind(status.st_mode), is_null, nonblock, access_mode(flags));
    }
    free(numbers);
}

static void report_process(FILE *report) {
    sigset_t blocked;
    sigemptyset(&blocked);
    sigprocmask(SIG_BLOCK, NULL, &blocked);
    int empty = 1;
    for (int number = 1; number < NSIG; number++) {
        if (sigismember(&blocked, number) == 1) {
            empty = 0;
        }
    }
    mode_t mask = umask(0);
    umask(mask);
    char cwd[4096];
    if (getcwd(cwd, sizeof(cwd)) == NULL) {
        strcpy(cwd, "unreadable");
    }
    fprintf(report, "cwd=%s\numask=%03o\nsigmask_empty=%d\nsid_is_self=%d\n", cwd, (unsigned)mask,
            empty, getsid(0) == getpid());
    for (int number = 1; number < NSIG; number++) {
        struct sigaction action;
        memset(&action, 0, sizeof(action));
        if (sigaction(number, NULL, &action) == 0 && action.sa_handler == SIG_IGN) {
            fprintf(report, "ignored=%d\n", number);
        }
    }
}

static void report_recipe(FILE *report) {
    struct sigaction alarm_action;
    memset(&alarm_action, 0, sizeof(alarm_action));
    alarm_action.sa_handler = on_alarm;
    sigemptyset(&alarm_action.sa_mask);
    if (sigaction(SIGALRM, &alarm_action, NULL) != 0) {
        _exit(BROKEN);
    }
    alarm(5);
    fputs("recipe_hex=", report);
    unsigned char buffer[4096];
    for (;;) {
        ssize_t got = read(RECIPE_FD, buffer, sizeof(buffer));
        if (got < 0 && errno == EINTR) {
            continue;
        }
        if (got < 0) {
            fprintf(report, "\nrecipe_error=%d\n", errno);
            return;
        }
        if (got == 0) {
            break;
        }
        for (ssize_t index = 0; index < got; index++) {
            fprintf(report, "%02x", buffer[index]);
        }
    }
    alarm(0);
    fputs("\nrecipe_eof=1\n", report);
}

static int record(int argc, char **argv) {
    FILE *report = fdopen(STDERR_FILENO, "w");
    if (report == NULL) {
        return BROKEN;
    }
    fprintf(report, "argc=%d\n", argc);
    for (int index = 0; index < argc; index++) {
        fprintf(report, "argv%d=%s\n", index, argv[index]);
    }
    int entries = 0;
    for (char **entry = environ; entry != NULL && *entry != NULL; entry++) {
        entries++;
    }
    fprintf(report, "envc=%d\n", entries);
    for (int index = 0; index < entries; index++) {
        const char *equals = strchr(environ[index], '=');
        int length = equals == NULL ? (int)strlen(environ[index]) : (int)(equals - environ[index]);
        fprintf(report, "env_name=%.*s\n", length, environ[index]);
    }
    report_process(report);
    report_descriptors(report);
    report_recipe(report);
    return fclose(report) == 0 ? 0 : BROKEN;
}

int main(int argc, char **argv) {
    if (argc < 1 || argv[0] == NULL) {
        return USAGE;
    }
    const char *slash = strrchr(argv[0], '/');
    const char *mode = slash == NULL ? argv[0] : slash + 1;
    if (strcmp(mode, "record") == 0) {
        return record(argc, argv);
    }
    return USAGE;
}
