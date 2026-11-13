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
#include <unistd.h>
extern "C" int vz_office_bootstrap(const char *);
extern "C" int vz_office_memory_probe(const char *,const char *);

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
    if (strcmp(argv[2], "control")) {
        int status = vz_office_bootstrap(profile.c_str());
        if (status) return status;
    }
    if (!strcmp(argv[2], "memory")) {
        if (argc != 6) return 64;
        return vz_office_memory_probe(argv[4], argv[5]);
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
    return ferror(stdout) ? 72 : 0;
}
