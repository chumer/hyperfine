#include <errno.h>
#include <fcntl.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <sys/wait.h>
#include <unistd.h>

int main(int argc, char **argv) {
    if (argc < 3) return 1;
    const char *mode = argv[1];
    size_t size = strtoull(argv[2], NULL, 10);
    if (strcmp(mode, "none") == 0) return 0;
    if (strcmp(mode, "child") == 0) {
        pid_t pid = fork();
        if (pid < 0) { perror("fork"); return 1; }
        if (pid > 0) {
            int status;
            while (waitpid(pid, &status, 0) < 0) {
                if (errno == EINTR) continue;
                perror("waitpid"); return 1;
            }
            return WIFEXITED(status) ? WEXITSTATUS(status) : 1;
        }
        mode = "alloc";
    }
    int file_backed = strcmp(mode, "read") == 0 || strcmp(mode, "cow") == 0;
    int fd = -1;
    if (file_backed) {
        if (argc != 4) return 1;
        fd = open(argv[3], O_RDONLY);
        if (fd < 0) { perror("open"); return 1; }
    }
    int protection = PROT_READ | (strcmp(mode, "read") == 0 ? 0 : PROT_WRITE);
    volatile unsigned char *memory = mmap(NULL, size, protection,
                                          MAP_PRIVATE | (file_backed ? 0 : MAP_ANON), fd, 0);
    if (memory == MAP_FAILED) { perror("mmap"); return 1; }
    if (fd >= 0 && close(fd)) { perror("close"); return 1; }
    for (size_t i = 0; i < size; i += (size_t)getpagesize()) {
        if (strcmp(mode, "read") == 0) (void)memory[i];
        else memory[i] = 1;
    }
    if (strcmp(mode, "free") == 0 && munmap((void *)memory, size)) {
        perror("munmap"); return 1;
    }
    return 0;
}
