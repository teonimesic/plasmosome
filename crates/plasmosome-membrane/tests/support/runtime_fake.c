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
#include <stddef.h>
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

#if defined(__APPLE__)
#include <sys/socket.h>
#include <sys/un.h>

extern int sandbox_init(const char *, unsigned int, char **);
extern void sandbox_free_error(char *);
#if defined(PLASMOSOME_FIXTURE_LIBRARY)
extern int plasmosome_fake(void);
#endif

#define RECIPE_CAP (1U << 20)

static char *read_bounded(int fd) {
    char *text = calloc(RECIPE_CAP + 2, 1);
    if (text == NULL) _exit(BROKEN);
    size_t used = 0;
    for (;;) {
        ssize_t got = read(fd, text + used, RECIPE_CAP + 1 - used);
        if (got < 0 && errno == EINTR) continue;
        if (got < 0) _exit(BROKEN);
        if (got == 0) return text;
        used += (size_t)got;
        if (used > RECIPE_CAP) _exit(BROKEN);
    }
}

static void field(const char *text, const char *key, char *out, size_t capacity) {
    const char *at = strstr(text, key);
    if (at == NULL) _exit(BROKEN);
    at += strlen(key);
    size_t used = 0;
    while (*at != '"') {
        unsigned char byte = (unsigned char)*at++;
        if (byte == 0 || used + 1 >= capacity) _exit(BROKEN);
        if (byte == '\\') {
            byte = (unsigned char)*at++;
            switch (byte) {
            case '"': case '\\': case '/': break;
            case 'n': byte = '\n'; break;
            case 'r': byte = '\r'; break;
            case 't': byte = '\t'; break;
            case 'b': byte = '\b'; break;
            case 'f': byte = '\f'; break;
            default: _exit(BROKEN);
            }
        }
        out[used++] = (char)byte;
    }
    out[used] = 0;
}

static void mark_started(const char *control) {
    char marker[4096];
    if (strlen(control) >= sizeof(marker)) _exit(BROKEN);
    strcpy(marker, control);
    for (int index = 0; index < 2; index++) {
        char *slash = strrchr(marker, '/');
        if (slash == NULL) _exit(BROKEN);
        *slash = 0;
    }
    size_t used = strlen(marker);
    int written = snprintf(marker + used, sizeof(marker) - used, "/markers/started-%d", getpid());
    if (written < 0 || (size_t)written >= sizeof(marker) - used) _exit(BROKEN);
    int fd = open(marker, O_WRONLY | O_CREAT | O_EXCL, 0600);
    if (fd < 0 || close(fd) != 0) _exit(BROKEN);
}

static int connect_lane(const char *path) {
    struct sockaddr_un address;
    memset(&address, 0, sizeof(address));
    address.sun_family = AF_UNIX;
    size_t length = strlen(path);
    if (length >= sizeof(address.sun_path)) _exit(BROKEN);
    memcpy(address.sun_path, path, length + 1);
    address.sun_len = (unsigned char)(offsetof(struct sockaddr_un, sun_path) + length + 1);
    int fd = socket(AF_UNIX, SOCK_STREAM, 0);
    if (fd < 0 || connect(fd, (struct sockaddr *)&address, address.sun_len) != 0) _exit(BROKEN);
    return fd;
}

static void hello(int fd, int id, const char *lane, const char *boot, const char *policy) {
    char expected[128], request[128], reply[256];
    int length = snprintf(expected, sizeof(expected),
        "{\"id\":%d,\"method\":\"hello\",\"params\":{\"lane\":\"%s\"}}\n", id, lane);
    if (length <= 0 || (size_t)length >= sizeof(expected)) _exit(BROKEN);
    size_t used = 0;
    while (used < (size_t)length) {
        ssize_t got = read(fd, request + used, (size_t)length - used);
        if (got < 0 && errno == EINTR) continue;
        if (got <= 0) _exit(80);
        used += (size_t)got;
    }
    if (memcmp(request, expected, used) != 0) _exit(80);
    length = snprintf(reply, sizeof(reply),
        "{\"id\":%d,\"result\":{\"boot\":\"%s\",\"policy\":\"%s\"}}\n", id, boot, policy);
    if (length <= 0 || (size_t)length >= sizeof(reply)) _exit(BROKEN);
    used = 0;
    while (used < (size_t)length) {
        ssize_t sent = write(fd, reply + used, (size_t)length - used);
        if (sent < 0 && errno == EINTR) continue;
        if (sent <= 0) _exit(BROKEN);
        used += (size_t)sent;
    }
}

static char worker_line[64];
static size_t worker_line_length;

static void report_worker(int signal) {
    (void)signal;
    int saved = errno;
    size_t used = 0;
    while (used < worker_line_length) {
        ssize_t sent = write(STDERR_FILENO, worker_line + used, worker_line_length - used);
        if (sent < 0 && errno == EINTR) continue;
        if (sent <= 0) break;
        used += (size_t)sent;
    }
    errno = saved;
}

static void hold_before_hello(const char *control) {
    char path[256];
    strcpy(path, control);
    char *slash = strrchr(path, '/');
    if (slash == NULL) _exit(BROKEN);
    strcpy(slash + 1, "hello-gate");
    int gate = open(path, O_RDONLY);
    if (gate < 0 && errno == ENOENT) return;
    if (gate < 0) _exit(BROKEN);
    snprintf(slash + 1, sizeof(path) - (size_t)(slash + 1 - path), "before-hello-%d", getpid());
    int witness = open(path, O_CREAT | O_EXCL | O_WRONLY, 0600);
    if (witness < 0 || close(witness) != 0) _exit(BROKEN);
    char release;
    ssize_t got;
    do { got = read(gate, &release, 1); } while (got < 0 && errno == EINTR);
    if (got != 1 || release != 'G' || close(gate) != 0) _exit(BROKEN);
}

static int shim(const char *mode) {
    char *recipe = read_bounded(RECIPE_FD);
    close(RECIPE_FD);
    char control[104], data[104], host[4096], policy[65], boot[65];
    field(recipe, "\"control_path\":\"", control, sizeof(control));
    field(recipe, "\"data_path\":\"", data, sizeof(data));
    field(recipe, "\"host_policy\":{\"path\":\"", host, sizeof(host));
    const char *guest = strstr(recipe, "\"guest_policy\":{");
    if (guest == NULL) _exit(BROKEN);
    field(guest, "\"sha256\":\"", policy, sizeof(policy));
    if (strlen(policy) != 64) _exit(BROKEN);
    free(recipe);
    mark_started(control);
    if (strcmp(mode, "exit-early") == 0) _exit(66);
    if (strcmp(mode, "silent") == 0) { for (;;) pause(); }
    int random = open("/dev/urandom", O_RDONLY);
    unsigned char bytes[32];
    size_t used = 0;
    while (used < sizeof(bytes)) {
        ssize_t got = read(random, bytes + used, sizeof(bytes) - used);
        if (got < 0 && errno == EINTR) continue;
        if (got <= 0) _exit(BROKEN);
        used += (size_t)got;
    }
    close(random);
    for (size_t index = 0; index < sizeof(bytes); index++) {
        snprintf(boot + index * 2, 3, "%02x", bytes[index]);
    }
    if (strcmp(mode, "shim-worker") == 0) {
        sigset_t blocked, previous;
        sigemptyset(&blocked);
        sigaddset(&blocked, SIGUSR1);
        if (sigprocmask(SIG_BLOCK, &blocked, &previous) != 0) _exit(BROKEN);
        struct sigaction action;
        memset(&action, 0, sizeof(action));
        action.sa_handler = report_worker;
        sigemptyset(&action.sa_mask);
        if (sigaction(SIGUSR1, &action, NULL) != 0) _exit(BROKEN);
        pid_t worker = fork();
        if (worker < 0) _exit(BROKEN);
        if (worker == 0) {
            const char prefix[] = "writer=";
            for (size_t index = 0; index < sizeof(prefix) - 1; index++) worker_line[worker_line_length++] = prefix[index];
            unsigned int pid = (unsigned int)getpid();
            char digits[16];
            size_t count = 0;
            do { digits[count++] = (char)('0' + pid % 10); pid /= 10; } while (pid != 0);
            while (count != 0) worker_line[worker_line_length++] = digits[--count];
            worker_line[worker_line_length++] = '\n';
            if (sigprocmask(SIG_SETMASK, &previous, NULL) != 0) _exit(BROKEN);
            for (;;) pause();
        }
        if (sigprocmask(SIG_SETMASK, &previous, NULL) != 0) _exit(BROKEN);
        dprintf(STDERR_FILENO, "worker=%d\n", worker);
    }
    if (strcmp(mode, "shim-unconfined") != 0) {
        int fd = open(host, O_RDONLY);
        if (fd < 0) _exit(67);
        char *profile = read_bounded(fd);
        close(fd);
        char *error = NULL;
        if (sandbox_init(profile, 0, &error) != 0) {
            if (error != NULL) { dprintf(STDERR_FILENO, "%s\n", error); sandbox_free_error(error); }
            _exit(67);
        }
        free(profile);
    }
    if (strcmp(mode, "shim-bad-policy") == 0) policy[0] = policy[0] == '0' ? '1' : '0';
    hold_before_hello(control);
    int normal = connect_lane(control);
    hello(normal, 1, "normal", boot, policy);
    int withdrawal = connect_lane(control);
    hello(withdrawal, 2, "withdrawal", boot, policy);
    int association = connect_lane(data);
    char buffer[256];
    for (;;) {
        ssize_t got = read(association, buffer, sizeof(buffer));
        if (got < 0 && errno == EINTR) continue;
        if (got <= 0) break;
    }
    for (;;) pause();
}
#endif
int main(int argc, char **argv) {
#if defined(PLASMOSOME_FIXTURE_LIBRARY)
    if (plasmosome_fake() != 0) return BROKEN;
#endif
    if (argc < 1 || argv[0] == NULL) {
        return USAGE;
    }
    const char *slash = strrchr(argv[0], '/');
    const char *mode = slash == NULL ? argv[0] : slash + 1;
    if (strcmp(mode, "record") == 0) {
        return record(argc, argv);
    }
#if defined(__APPLE__)
    const char *modes[] = {"shim", "shim-unconfined", "shim-worker", "shim-bad-policy", "exit-early", "silent"};
    for (size_t index = 0; index < sizeof(modes) / sizeof(modes[0]); index++) {
        if (strcmp(mode, modes[index]) == 0) return shim(mode);
    }
#endif
    return USAGE;
}
