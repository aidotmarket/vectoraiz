/* Isolated minimal helper: address-growth experiment, never a resident-cap claim.
 * XNU rationale: xnu-11417.101.15/bsd/kern/kern_resource.c.
 * No RSS sampler. All startup opens precede confinement/readiness. */
#include <mach/mach.h>
#include <mach/mach_vm.h>
#include <libproc.h>
#include <sys/resource.h>
#include <sys/mman.h>
#include <sys/stat.h>
#include <fcntl.h>
#include <unistd.h>
#include <errno.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <dlfcn.h>
#include <sandbox.h>
#define GiB (1024ULL*1024*1024)
#define BUDGET (1536ULL*1024*1024)
static void touch(void *p,size_t n) {volatile char *b=p;for(size_t i=0;i<n;i+=16384)b[i]=1;}
int main(int argc,char **argv) {
    if(argc!=6) return 64; /* profile, kind, scratch, role, pdfium */
    const char *kind=argv[2],*role=argv[4];
    if(!strcmp(role,"ocr")) {puts("{\"status\":\"blocked\",\"reason\":\"offline OCR engine/assets unavailable\"}");return 78;}
    void *library=dlopen(argv[5],RTLD_NOW|RTLD_LOCAL);
    if(!library)return 65;
    void (*init)(void)=dlsym(library,"FPDF_InitLibrary");if(!init)return 65;init();
    FILE *profile_file=fopen(argv[1],"r");if(!profile_file)return 66;
    char profile[16384]={0};if(fread(profile,1,sizeof(profile)-1,profile_file)==sizeof(profile)-1)return 66;fclose(profile_file);
    int fd=open(argv[3],O_CREAT|O_EXCL|O_RDWR,0600);if(fd<0)return 67;
    if(ftruncate(fd,2*GiB))return 67;
    void *reservation=NULL;
    if(!strcmp(kind,"replacement")) {
        // Shared O_RDONLY mapping: max protection does not grant writable COW.
        close(fd);fd=open(argv[3],O_RDONLY);if(fd<0)return 67;
        reservation=mmap(NULL,2*GiB,PROT_READ,MAP_SHARED,fd,0);
    } else if(!strcmp(kind,"reserved-dirty")) reservation=mmap(NULL,2*GiB,PROT_READ|PROT_WRITE,MAP_ANON|MAP_PRIVATE,-1,0);
    if(reservation==MAP_FAILED)return 68;
    // Region list is durable evidence on stderr. Count max-write mappings,
    // including presently read-only COW, rather than only current protections.
    mach_vm_address_t address=0;uint64_t dirtyable=0,readonly=0,writable=0;int regions=0;
    for(;;) {
        mach_vm_size_t size=0;vm_region_basic_info_data_64_t info={0};
        mach_msg_type_number_t count=VM_REGION_BASIC_INFO_COUNT_64;mach_port_t object=0;
        kern_return_t kr=mach_vm_region(mach_task_self(),&address,&size,VM_REGION_BASIC_INFO_64,(vm_region_info_t)&info,&count,&object);
        if(object)mach_port_deallocate(mach_task_self(),object);
        if(kr || !size)break;
        if(info.max_protection&VM_PROT_WRITE)dirtyable+=size;else readonly+=size;
        if(info.protection&VM_PROT_WRITE)writable+=size;
        fprintf(stderr,"{\"region\":%d,\"address\":%llu,\"bytes\":%llu,\"protection\":%d,\"max_protection\":%d,\"shared\":%s,\"reserved\":%s}\n",regions++,address,size,info.protection,info.max_protection,info.shared?"true":"false",info.reserved?"true":"false");
        if(address+size<address)break;address+=size;
    }
    struct proc_taskinfo task={0};if(proc_pidinfo(getpid(),PROC_PIDTASKINFO,0,&task,sizeof(task))!=sizeof(task))return 69;
    uint64_t remaining=dirtyable<BUDGET?BUDGET-dirtyable:0;
    struct rlimit limit={task.pti_virtual_size+remaining,task.pti_virtual_size+remaining},actual={0};
    errno=0;int rc=setrlimit(RLIMIT_AS,&limit),error=errno;if(getrlimit(RLIMIT_AS,&actual))return 70;
    fprintf(stderr,"{\"baseline_mapped_bytes\":%llu,\"dirtyable_baseline_bytes\":%llu,\"current_writable_bytes\":%llu,\"excluded_readonly_bytes\":%llu,\"remaining_budget_bytes\":%llu,\"role_budget_bytes\":%llu,\"requested_limit\":%llu,\"soft\":%llu,\"hard\":%llu,\"set_rc\":%d,\"errno\":%d,\"baseline_rss_bytes\":%llu,\"sampler_enabled\":false,\"resident_budget_proven\":false}\n",task.pti_virtual_size,dirtyable,writable,readonly,remaining,BUDGET,limit.rlim_cur,actual.rlim_cur,actual.rlim_max,rc,error,task.pti_resident_size);
    if(rc || actual.rlim_cur!=limit.rlim_cur || actual.rlim_max!=limit.rlim_max)return 71;
    if(dirtyable>BUDGET) {puts("{\"status\":\"fail\",\"reason\":\"dirtyable baseline exceeds role budget; job refused before input\"}");return 72;}
    char *message=NULL;if(sandbox_init(profile,0,&message))return 73;
    struct rlimit zero={0,0},z={0};if(setrlimit(RLIMIT_NOFILE,&zero) || getrlimit(RLIMIT_NOFILE,&z) || z.rlim_cur || z.rlim_max)return 74;
    fprintf(stderr,"{\"ready\":true,\"nofile_soft\":%llu,\"nofile_hard\":%llu}\n",z.rlim_cur,z.rlim_max);
    size_t total=0,step=!strcmp(kind,"many-small")?65536:64*1024*1024;int mach_error=0;void *p=NULL;rc=-1;errno=0;
    if(!strcmp(kind,"replacement")) {
        if(munmap(reservation,2*GiB))return 75;
        p=mmap(NULL,2*GiB,PROT_READ|PROT_WRITE,MAP_PRIVATE|MAP_ANON,-1,0);error=errno;rc=p==MAP_FAILED?-1:0;
        if(!rc) {touch(p,2*GiB);total=2*GiB;}
    } else if(!strcmp(kind,"burst")) {p=mmap(NULL,2*GiB,PROT_READ|PROT_WRITE,MAP_PRIVATE|MAP_ANON,-1,0);error=errno;rc=p==MAP_FAILED?-1:0;}
    else if(!strcmp(kind,"mapped-file")) {p=mmap(NULL,2*GiB,PROT_READ|PROT_WRITE,MAP_SHARED,fd,0);error=errno;rc=p==MAP_FAILED?-1:0;}
    else if(!strcmp(kind,"raise")) {limit.rlim_cur+=BUDGET;limit.rlim_max+=BUDGET;rc=setrlimit(RLIMIT_AS,&limit);error=errno;}
    else {
        while(total<=remaining+step) {
            errno=0;
            if(!strcmp(kind,"mach")) {mach_vm_address_t a=0;mach_error=mach_vm_allocate(mach_task_self(),&a,step,VM_FLAGS_ANYWHERE);rc=mach_error?-1:0;}
            else if(!strcmp(kind,"malloc")) {p=malloc(step);rc=p?0:-1;if(p)touch(p,step);}
            else {p=mmap(NULL,step,PROT_READ|PROT_WRITE,MAP_PRIVATE|MAP_ANON,-1,0);rc=p==MAP_FAILED?-1:0;}
            error=errno;if(rc)break;total+=step;
        }
    }
    struct rusage usage={0};getrusage(RUSAGE_SELF,&usage);
    printf("{\"rc\":%d,\"errno\":%d,\"mach_error\":%d,\"cumulative_new_bytes\":%zu,\"step_bytes\":%zu,\"peak_rss_bytes\":%ld,\"resident_budget_exceeded\":%s}\n",rc,error,mach_error,total,step,usage.ru_maxrss,total+dirtyable>BUDGET?"true":"false");
    return 0;
}
