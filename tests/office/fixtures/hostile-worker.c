/* C1 synthetic capability probes, one bounded process per operation. */
#include <arpa/inet.h>
#include <CoreServices/CoreServices.h>
#include <dirent.h>
#include <errno.h>
#include <fcntl.h>
#include <mach/mach.h>
#include <netdb.h>
#include <net/if.h>
#include <servers/bootstrap.h>
#include <spawn.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/resource.h>
#include <sys/socket.h>
#include <sys/stat.h>
#include <sys/un.h>
#include <sys/wait.h>
#include <unistd.h>
#include <xpc/xpc.h>

int main(int argc, char **argv) {
    if (argc != 5) return 2;
    signal(SIGPIPE, SIG_IGN);
    struct rlimit files={64,64}, cpu={2,2}, core={0,0};
    #ifndef VZ_DESCRIPTOR_ONLY
    if (setrlimit(RLIMIT_NOFILE,&files)) return 4;
#endif
    if (setrlimit(RLIMIT_CPU,&cpu) || setrlimit(RLIMIT_CORE,&core)) return 4;
    const char *probe = argv[1], *snapshot = argv[2], *host = argv[3];
    int rc = -1, fd = -1, created = 0;
    errno = 0;
    if (!strcmp(probe,"memory_budget")) {
        size_t bytes=2ULL*1024*1024*1024;
        volatile char *allocation=malloc(bytes);
        if(allocation) {for(size_t i=0;i<bytes;i+=4096) allocation[i]=1;rc=0;free((void *)allocation);}
    } else if (!strcmp(probe,"snapshot") || !strcmp(probe,"host_read")) {
        #ifdef VZ_DESCRIPTOR_ONLY
        extern int vz_office_input_fd(void);
        if(!strcmp(probe,"snapshot")) fd=vz_office_input_fd();else
#endif
        fd = open(!strcmp(probe,"snapshot") ? snapshot : host,O_RDONLY);
        if (fd >= 0) { char byte; rc = (int)read(fd,&byte,1); }
    } else if (!strcmp(probe,"host_stat")) {
        struct stat s; rc = stat(host,&s);
    } else if (!strcmp(probe,"host_list")) {
        DIR *d = opendir(host); if (d) { rc = 0; closedir(d); }
    } else if (!strcmp(probe,"host_write") || !strcmp(probe,"snapshot_write")) {
        fd = open(!strcmp(probe,"snapshot_write") ? snapshot : host,O_WRONLY);
        if (fd >= 0) rc = (int)write(fd,"X",1);
    } else if (!strcmp(probe,"host_create") || !strcmp(probe,"scratch_create")) {
        #ifdef VZ_DESCRIPTOR_ONLY
        extern int vz_office_scratch_fd(void);
        if(!strcmp(probe,"scratch_create")) {fd=vz_office_scratch_fd();rc=(int)write(fd,"C1",2);} else
#endif
        {fd = open(host,O_CREAT|O_EXCL|O_WRONLY,0600); rc = fd;}
    } else if (!strcmp(probe,"host_rename")) { rc = rename(host,argv[4]);
    } else if (!strcmp(probe,"host_unlink")) { rc = unlink(host);
    } else if (!strcmp(probe,"child_exec")) {
        char *args[] = {"/usr/bin/true",NULL}; char *env[] = {NULL};
        pid_t pid; int error = posix_spawn(&pid,args[0],NULL,NULL,args,env);
        if (error) { errno=error; rc=-1; } else { waitpid(pid,&rc,0); rc=0; }
    } else if (!strcmp(probe,"child_fork")) {
        pid_t child=fork();
        if (child==0) _exit(0);
        if (child>0) {int status;waitpid(child,&status,0);rc=0;}
    } else if (!strcmp(probe,"open_url")) {
        CFURLRef url=CFURLCreateWithString(NULL,CFSTR("c1-feasibility-probe://blocked"),NULL);
#pragma clang diagnostic push
#pragma clang diagnostic ignored "-Wdeprecated-declarations"
        OSStatus error=LSOpenCFURLRef(url,NULL);
#pragma clang diagnostic pop
        CFRelease(url);
        printf("{\"rc\":%d,\"errno\":%d,\"ls_error\":%d,\"explicit_launch_denial\":%s}\n",error ? -1 : 0,errno,(int)error,error==kLSNoLaunchPermissionErr ? "true":"false");
        return 0;
    } else if (!strcmp(probe,"inherited_host_read")) {
        char byte; rc=(int)read(atoi(argv[4]),&byte,1);
    } else if (!strcmp(probe,"inherited_handle")) {
        char byte; rc=(int)read(9,&byte,1);
    } else if (!strcmp(probe,"dns")) {
        struct addrinfo *a=NULL;
        int error=getaddrinfo("c1-probe.invalid",NULL,NULL,&a);
        if (a) freeaddrinfo(a);
        printf("{\"rc\":%d,\"errno\":%d,\"gai_error\":%d}\n",error ? -1 : 0,errno,error);
        return 0;
    } else if (!strncmp(probe,"mach_",5)) {
        mach_port_t service=MACH_PORT_NULL;
        const char *name = !strcmp(probe,"mach_open_url") ? "com.apple.coreservices.launchservicesd" : "com.apple.cfprefsd.daemon";
        kern_return_t error=bootstrap_look_up(bootstrap_port,name,&service);
        if (service != MACH_PORT_NULL) mach_port_deallocate(mach_task_self(),service);
        printf("{\"rc\":%d,\"errno\":%d,\"mach_error\":%d}\n",error==KERN_SUCCESS ? 0 : -1,errno,error);
        return 0;
    } else if (!strcmp(probe,"xpc_broker")) {
        /* XPC collapses denied/absent endpoints into generic errors. Preserve
         * an actual kernel lookup of the identical endpoint in this process. */
        mach_port_t endpoint=MACH_PORT_NULL;
        kern_return_t lookup=bootstrap_look_up(bootstrap_port,"com.apple.cfprefsd.daemon",&endpoint);
        if(endpoint!=MACH_PORT_NULL) mach_port_deallocate(mach_task_self(),endpoint);
        xpc_connection_t c=xpc_connection_create_mach_service("com.apple.cfprefsd.daemon",NULL,0);
        xpc_connection_set_event_handler(c, ^(xpc_object_t event) { (void)event; });
        xpc_connection_resume(c);
        xpc_object_t m=xpc_dictionary_create(NULL,NULL,0);
        xpc_dictionary_set_string(m,"C1","synthetic-read-only-probe");
        xpc_object_t reply=xpc_connection_send_message_with_reply_sync(c,m);
        int denied = xpc_get_type(reply)==XPC_TYPE_ERROR;
        char *desc=xpc_copy_description(reply);
        printf("{\"rc\":%d,\"errno\":%d,\"xpc_error\":%s,\"endpoint_lookup_error\":%d,\"endpoint_kernel_denial\":%s}\n",denied ? -1 : 0,errno,denied ? "true":"false",lookup,lookup==BOOTSTRAP_NOT_PRIVILEGED?"true":"false");
        free(desc); xpc_release(reply); xpc_release(m); xpc_connection_cancel(c); xpc_release(c);
        return 0;
    } else if (!strcmp(probe,"unix_connect")) {
        fd=socket(AF_UNIX,SOCK_STREAM,0); created=fd>=0;
        struct sockaddr_un a={0}; a.sun_family=AF_UNIX;
        strlcpy(a.sun_path,host,sizeof(a.sun_path));
        if (fd>=0) rc=connect(fd,(struct sockaddr *)&a,sizeof(a));
    } else if (!strcmp(probe,"system_socket")) {
        fd=socket(AF_SYSTEM,SOCK_DGRAM,2); created=fd>=0; rc=fd;
    } else if (!strcmp(probe,"socketpair")) {
        int pair[2]; rc=socketpair(AF_UNIX,SOCK_STREAM,0,pair);
        if (!rc) {created=1;close(pair[0]);close(pair[1]);}
    } else {
        char operation[40], destination[40]; int family=0, udp=0;
        if (sscanf(probe,"%39[^:]:%d:%d:%39s",operation,&family,&udp,destination)!=4) return 3;
        int domain=family==6 ? AF_INET6 : AF_INET;
        fd=socket(domain,udp ? SOCK_DGRAM : SOCK_STREAM,0); created=fd>=0;
        struct timeval timeout={1,0};
        if (fd>=0) {
            setsockopt(fd,SOL_SOCKET,SO_SNDTIMEO,&timeout,sizeof(timeout));
            struct sockaddr_storage storage={0}; socklen_t len;
            if (family==6) {
                struct sockaddr_in6 *a=(void *)&storage; a->sin6_family=AF_INET6;
                a->sin6_port=htons((unsigned short)atoi(argv[4]));
                char *zone=strchr(destination,'%');
                if(zone) {*zone=0; a->sin6_scope_id=if_nametoindex(zone+1);}
                if(inet_pton(AF_INET6,destination,&a->sin6_addr)!=1) return 5; len=sizeof(*a);
            } else {
                struct sockaddr_in *a=(void *)&storage; a->sin_family=AF_INET;
                a->sin_port=htons((unsigned short)atoi(argv[4]));
                inet_pton(AF_INET,destination,&a->sin_addr); len=sizeof(*a);
            }
            if (!strcmp(operation,"socket")) rc=fd;
            else if (!strcmp(operation,"connect")) rc=connect(fd,(void *)&storage,len);
            else if (!strcmp(operation,"bind")) rc=bind(fd,(void *)&storage,len);
            else if (!strcmp(operation,"listen")) rc=listen(fd,1);
            else if (!strcmp(operation,"sendto")) rc=(int)sendto(fd,"C1",2,0,(void *)&storage,len);
            else if (!strcmp(operation,"sendmsg")) {
                struct iovec iov={"C1",2}; struct msghdr m={0};
                m.msg_name=&storage;m.msg_namelen=len;m.msg_iov=&iov;m.msg_iovlen=1;
                rc=(int)sendmsg(fd,&m,0);
            } else if (!strcmp(operation,"connectx")) {
                sa_endpoints_t e={0}; e.sae_dstaddr=(void *)&storage;e.sae_dstaddrlen=len;
                rc=connectx(fd,&e,SAE_ASSOCID_ANY,0,NULL,0,NULL,NULL);
            } else if (!strcmp(operation,"multicast")) {
                if(family==6) {struct ipv6_mreq m={0};inet_pton(AF_INET6,"ff02::1",&m.ipv6mr_multiaddr);m.ipv6mr_interface=if_nametoindex("lo0");rc=setsockopt(fd,IPPROTO_IPV6,IPV6_JOIN_GROUP,&m,sizeof(m));}
                else {struct ip_mreq m={0};inet_pton(AF_INET,"224.0.0.251",&m.imr_multiaddr);m.imr_interface.s_addr=htonl(INADDR_LOOPBACK);rc=setsockopt(fd,IPPROTO_IP,IP_ADD_MEMBERSHIP,&m,sizeof(m));}
            }
        }
    }
    int error=errno;
    if (fd>=0) close(fd);
    printf("{\"rc\":%d,\"errno\":%d,\"socket_created\":%s}\n",rc,error,created ? "true":"false");
    return 0;
}
