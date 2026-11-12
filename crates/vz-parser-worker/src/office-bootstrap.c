/* Trusted bootstrap used by actual ingest/render and separately launched OCR. */
#include <libproc.h>
#include <mach/mach.h>
#include <mach/mach_vm.h>
#include <sandbox.h>
#include <sys/mman.h>
#define main vz_office_probe_main
#include "../../../tests/office/fixtures/hostile-worker.c"
#undef main

int vz_office_bootstrap(const char *profile) {
    struct proc_taskinfo task = {0};
    if (proc_pidinfo(getpid(),PROC_PIDTASKINFO,0,&task,sizeof(task)) != sizeof(task)) return 70;
    uint64_t budget=1536ULL*1024*1024;
    struct rlimit limit={task.pti_virtual_size+budget,task.pti_virtual_size+budget}, readback={0};
    int rc=setrlimit(RLIMIT_AS,&limit), error=errno;
    if (getrlimit(RLIMIT_AS,&readback)) return 71;
    fprintf(stderr,"{\"baseline_mapped_bytes\":%llu,\"role_budget_bytes\":%llu,\"requested_limit\":%llu,\"soft\":%llu,\"hard\":%llu,\"set_rc\":%d,\"errno\":%d,\"sampler_enabled\":false}\n",task.pti_virtual_size,budget,limit.rlim_cur,readback.rlim_cur,readback.rlim_max,rc,error);
    if (rc || readback.rlim_cur!=limit.rlim_cur || readback.rlim_max!=limit.rlim_max) return 72;
    /* Do not inherit host files/sockets into the confined role. */
    int maxfd=getdtablesize();
    for(int fd=3;fd<maxfd;fd++) close(fd);
    char *message=NULL;
    if (sandbox_init(profile,0,&message)) {
        fprintf(stderr,"sandbox init: %s\n",message ? message : "unknown");
        if(message) sandbox_free_error(message);
        return 73;
    }
    return 0;
}

int vz_office_memory_probe(const char *kind,const char *scratch) {
    const size_t budget=1536ULL*1024*1024, step=!strcmp(kind,"many-small") ? 65536 : 64*1024*1024;
    size_t total=0; int error=0, kernel=0, rc=-1;
    if(!strcmp(kind,"raise")) {
        struct rlimit r;getrlimit(RLIMIT_AS,&r);r.rlim_cur+=budget;r.rlim_max+=budget;
        rc=setrlimit(RLIMIT_AS,&r);error=errno;
    } else if(!strcmp(kind,"burst")) {
        void *p=mmap(NULL,2ULL*1024*1024*1024,PROT_READ|PROT_WRITE,MAP_PRIVATE|MAP_ANON,-1,0);
        rc=p==MAP_FAILED ? -1:0;error=errno;
    } else if(!strcmp(kind,"mapped-file")) {
        int fd=open(scratch,O_RDWR|O_CREAT,0600);
        if(fd<0) {fprintf(stderr,"scratch setup failed\n");return 74;}
        if(ftruncate(fd,2ULL*1024*1024*1024)) return 74;
        void *p=mmap(NULL,2ULL*1024*1024*1024,PROT_READ|PROT_WRITE,MAP_SHARED,fd,0);
        rc=p==MAP_FAILED ? -1:0;error=errno;close(fd);
    } else {
        while(total <= budget+step) {
            if(!strcmp(kind,"mach")) {
                mach_vm_address_t address=0;
                kernel=mach_vm_allocate(mach_task_self(),&address,step,VM_FLAGS_ANYWHERE);
                if(kernel!=KERN_SUCCESS) {rc=-1;break;}
            } else if(!strcmp(kind,"malloc")) {
                volatile char *p=malloc(step);
                if(!p) {error=errno;rc=-1;break;}
                for(size_t i=0;i<step;i+=4096) p[i]=1;
            } else {
                void *p=mmap(NULL,step,PROT_READ|PROT_WRITE,MAP_ANON|MAP_PRIVATE,-1,0);
                if(p==MAP_FAILED) {error=errno;rc=-1;break;}
            }
            total+=step;rc=0;
        }
    }
    fprintf(stdout,"{\"probe\":\"%s\",\"rc\":%d,\"errno\":%d,\"mach_error\":%d,\"cumulative_new_bytes\":%zu,\"step_bytes\":%zu}\n",kind,rc,error,kernel,total,step);
    return 0;
}
