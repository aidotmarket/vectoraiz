// Harness only: fixed parent-launched offline OCR with no path broker.
#include <tesseract/baseapi.h>
#include <leptonica/allheaders.h>
#include <clocale>
#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <fstream>
#include <iterator>
#include <string>
#include <vector>
#include <fcntl.h>
#include <sys/resource.h>
#include <mach/mach.h>
#include <unistd.h>
extern "C" int vz_office_bootstrap(const char *);
extern "C" int vz_office_memory_probe(const char *,const char *);
extern "C" int vz_office_reserve_baseline(void);
extern "C" void vz_office_supply_fds(int,int,int);
extern "C" int vz_office_socket_checks(int);

static void accounting() {
    struct rusage usage={};int rc=getrusage(RUSAGE_SELF,&usage);
    task_vm_info_data_t info={};mach_msg_type_number_t count=TASK_VM_INFO_COUNT;
    kern_return_t kr=task_info(mach_task_self(),TASK_VM_INFO,(task_info_t)&info,&count);
    fprintf(stderr,"{\"peak_rss_bytes\":%ld,\"rusage_rc\":%d,\"phys_footprint_bytes\":%llu,\"task_info_rc\":%d}\n",usage.ru_maxrss,rc,info.phys_footprint,kr);
}

static bool read_all(void *data, size_t size) {
    return fread(data, 1, size, stdin) == size;
}
int main(int argc, char **argv) {
    if (argc < 4) return 64;
    std::ifstream file(argv[1]);
    std::string profile((std::istreambuf_iterator<char>(file)), {});
    if (profile.empty()) return 65;
    setenv("OMP_THREAD_LIMIT", "1", 1);
    setlocale(LC_ALL, "C");
    tesseract::TessBaseAPI api;
    // All models and linked libraries load before confinement and before input.
    if (api.Init(argv[3], "eng+deu+fra+spa+jpn", tesseract::OEM_LSTM_ONLY)) return 66;
    api.SetPageSegMode(tesseract::PSM_AUTO);
    int supplied=-1;
    if(!strcmp(argv[2],"memory") && argc==6) {
        if(!strcmp(argv[4],"mapped-file")) {
            supplied=open(argv[5],O_RDWR|O_CREAT|O_EXCL,0600);
            if(supplied<0)return 73;
            vz_office_supply_fds(-1,supplied,-1);
        }
        if(!strcmp(argv[4],"reserved-dirty") && vz_office_reserve_baseline())return 74;
    }
    if(strstr(argv[2],"sockets")) {
        supplied=open("/dev/null",O_RDONLY);
        if(supplied<0)return 75;
        vz_office_supply_fds(supplied,-1,-1);
    }
    if (strcmp(argv[2], "control") && strcmp(argv[2], "control-sockets")) {
        int status = vz_office_bootstrap(profile.c_str());
        if (status) {accounting();return status;}
    }
    if(strstr(argv[2],"sockets")) {int rc=vz_office_socket_checks(supplied);accounting();return rc;}
    if (!strcmp(argv[2], "memory")) {
        if (argc != 6) return 64;
        int rc=vz_office_memory_probe(argv[4], argv[5]);accounting();return rc;
    }
    uint32_t ready_size = 14;
    fwrite(&ready_size, 4, 1, stdout);
    fwrite("{\"ready\":true}", 1, ready_size, stdout); fflush(stdout);
    uint32_t dimensions[2];
    if (!read_all(dimensions, sizeof(dimensions))) return 67;
    uint32_t width=dimensions[0], height=dimensions[1];
    if (!width || !height || width>4096 || height>4096) return 68;
    std::vector<unsigned char> rgb(size_t(width)*height*3);
    if (!read_all(rgb.data(), rgb.size())) return 69;
    api.SetImage(rgb.data(), width, height, 3, width*3);
    api.SetSourceResolution(150);
    char *text=api.GetUTF8Text();
    if (!text) return 70;
    size_t size=strlen(text);
    if (size>1024*1024) {delete[] text;return 71;}
    fwrite(text, 1, size, stdout);delete[] text;
    accounting();
    return ferror(stdout) ? 72 : 0;
}
