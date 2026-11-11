/* Synthetic C1 ingest/render capability probe. No tenant data or parser. */
#include <arpa/inet.h>
#include <errno.h>
#include <fcntl.h>
#include <stdio.h>
#include <stdlib.h>
#include <sys/socket.h>
#include <unistd.h>

static void read_probe(const char *name, const char *path) {
    int fd = open(path, O_RDONLY);
    int error = errno;
    char byte;
    if (fd >= 0) {
        if (read(fd, &byte, 1) >= 0) {
            printf("\"%s\":\"allowed\",", name);
            close(fd);
            return;
        }
        error = errno;
        close(fd);
    }
    printf("\"%s\":{\"denied_errno\":%d},", name, error);
}

int main(int argc, char **argv) {
    if (argc != 4) return 2;
    printf("{");
    read_probe("snapshot", argv[1]);
    read_probe("sentinel", argv[2]);
    int fd = socket(AF_INET, SOCK_STREAM, 0);
    int error = errno;
    int created = fd >= 0;
    if (created) {
        struct sockaddr_in address = {0};
        address.sin_family = AF_INET;
        address.sin_port = htons((unsigned short)atoi(argv[3]));
        inet_pton(AF_INET, "127.0.0.1", &address.sin_addr);
        if (connect(fd, (struct sockaddr *)&address, sizeof(address)) == 0) {
            printf("\"socket\":\"allowed\",\"socket_created\":true}\n");
            close(fd);
            return 0;
        }
        error = errno;
        close(fd);
    }
    printf("\"socket\":{\"denied_errno\":%d},\"socket_created\":%s}\n",
           error, created ? "true" : "false");
    return 0;
}
