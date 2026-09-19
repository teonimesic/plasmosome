#include <sys/types.h>
#include <sys/sysctl.h>
#include <sys/proc.h>
#include <sys/user.h>

#include <errno.h>
#include <stdint.h>
#include <stdlib.h>

static int table_certifies(const struct kinfo_proc *entries, size_t bytes,
                           size_t capacity, pid_t leader) {
    if (entries == NULL || leader <= 0 || bytes == 0 || bytes > capacity ||
        bytes % sizeof(*entries) != 0) {
        return 0;
    }

    size_t count = bytes / sizeof(*entries);
    size_t leaders = 0;
    for (size_t index = 0; index < count; index++) {
        const struct kinfo_proc *entry = &entries[index];
        if (entry->kp_proc.p_pid <= 0 || entry->kp_eproc.e_pgid != leader ||
            entry->kp_proc.p_stat != SZOMB) {
            return 0;
        }
        if (entry->kp_proc.p_pid == leader) {
            leaders++;
        }
    }
    return leaders == 1;
}

static int allocation_size_valid(size_t needed) {
    return needed != 0 && needed % sizeof(struct kinfo_proc) == 0;
}

int plasmosome_darwin_group_all_zombies(pid_t leader) {
    int mib[4] = {CTL_KERN, KERN_PROC, KERN_PROC_PGRP, leader};
    struct kinfo_proc entry;
    size_t bytes = sizeof(entry);

    if (sysctl(mib, 4, &entry, &bytes, NULL, 0) == 0) {
        return table_certifies(&entry, bytes, sizeof(entry), leader);
    }
    if (errno != ENOMEM) {
        return 0;
    }

    size_t needed = 0;
    if (sysctl(mib, 4, NULL, &needed, NULL, 0) != 0 ||
        !allocation_size_valid(needed)) {
        return 0;
    }

    struct kinfo_proc *entries = malloc(needed);
    if (entries == NULL) {
        return 0;
    }
    bytes = needed;
    int queried = sysctl(mib, 4, entries, &bytes, NULL, 0);
    int certified = queried == 0 &&
                    table_certifies(entries, bytes, needed, leader);
    free(entries);
    return certified;
}

int plasmosome_darwin_group_test_table(int scenario, pid_t leader) {
    struct kinfo_proc entries[3] = {0};
    for (size_t index = 0; index < 3; index++) {
        entries[index].kp_proc.p_pid = leader + (pid_t)index;
        entries[index].kp_eproc.e_pgid = leader;
        entries[index].kp_proc.p_stat = SZOMB;
    }

    switch (scenario) {
    case 0:
        return table_certifies(entries, sizeof(entries[0]),
                               sizeof(entries), leader);
    case 1:
        return table_certifies(entries, sizeof(entries),
                               sizeof(entries), leader);
    case 2:
        return table_certifies(entries, 0, sizeof(entries), leader);
    case 3:
        return table_certifies(entries, sizeof(entries) - 1,
                               sizeof(entries), leader);
    case 4:
        return table_certifies(entries, sizeof(entries) + 1,
                               sizeof(entries), leader);
    case 5:
        entries[0].kp_proc.p_pid = 0;
        return table_certifies(entries, sizeof(entries[0]),
                               sizeof(entries), leader);
    case 6:
        entries[0].kp_eproc.e_pgid = leader + 1;
        return table_certifies(entries, sizeof(entries[0]),
                               sizeof(entries), leader);
    case 7:
        entries[0].kp_proc.p_stat = SRUN;
        return table_certifies(entries, sizeof(entries[0]),
                               sizeof(entries), leader);
    case 8:
        entries[0].kp_proc.p_pid = leader + 1;
        return table_certifies(entries, sizeof(entries[0]),
                               sizeof(entries), leader);
    case 9:
        entries[1].kp_proc.p_pid = leader;
        return table_certifies(entries, sizeof(entries),
                               sizeof(entries), leader);
    case 10:
        return allocation_size_valid(0);
    case 11:
        return allocation_size_valid(sizeof(entries[0]) - 1);
    case 12:
        return allocation_size_valid(sizeof(entries[0]));
    case 13:
        return allocation_size_valid(SIZE_MAX);
    default:
        return 0;
    }
}
