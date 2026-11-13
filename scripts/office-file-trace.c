/* Diagnostic-only dyld interposer; never included in a qualified worker. */
#include <errno.h>
#include <dirent.h>
#include <fcntl.h>
#include <stdarg.h>
#include <stdio.h>
#include <string.h>
#include <unistd.h>

static void trace(const char *operation, const char *path, int result, int error) {
    char line[4096];
    int size=snprintf(line,sizeof(line),"FILE_TRACE pid=%d %s rc=%d errno=%d path=%s\n",getpid(),operation,result,error,path);
    if(size>0) write(STDERR_FILENO,line,(size_t)size<sizeof(line)?(size_t)size:sizeof(line)-1);
}
static int traced_open(const char *path,int flags,...) {
    mode_t mode=0;
    if(flags&O_CREAT) {va_list args;va_start(args,flags);mode=va_arg(args,int);va_end(args);}
    int fd=open(path,flags,mode),error=errno;
    trace("open",path,fd,error);errno=error;return fd;
}
static FILE *traced_fopen(const char *path,const char *mode) {
    FILE *file=fopen(path,mode);int error=errno;
    trace("fopen",path,file?fileno(file):-1,error);errno=error;return file;
}
static int traced_openat(int dir,const char *path,int flags,...) {
    mode_t mode=0;
    if(flags&O_CREAT) {va_list args;va_start(args,flags);mode=va_arg(args,int);va_end(args);}
    int fd=openat(dir,path,flags,mode),error=errno;
    trace("openat",path,fd,error);errno=error;return fd;
}
static DIR *traced_opendir(const char *path) {
    DIR *dir=opendir(path);int error=errno;
    trace("opendir",path,dir?dirfd(dir):-1,error);errno=error;return dir;
}
#define INTERPOSE(replacement, original) \
    __attribute__((used)) static struct {const void *replacement;const void *original;} \
    pair_##original __attribute__((section("__DATA,__interpose"))) = {(const void *)replacement,(const void *)original};
INTERPOSE(traced_open,open)
INTERPOSE(traced_fopen,fopen)
INTERPOSE(traced_openat,openat)
INTERPOSE(traced_opendir,opendir)
