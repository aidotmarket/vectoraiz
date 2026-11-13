/* Harness-only byte-backed PDFium v1 system font interface.
 * ABI: public/fpdf_sysfontinfo.h at PDFium chromium/7350. No OS mapper.
 * All font bytes are bounded, hash-checked and resident before bootstrap.
 * Handles and callbacks live until process exit, after Pdfium destruction. */
#include <CommonCrypto/CommonDigest.h>
#include <dlfcn.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <strings.h>

typedef struct Info Info;
struct Info {
    int version;
    void (*release)(Info *);
    void (*enumerate)(Info *, void *);
    void *(*map)(Info *, int, int, int, int, const char *, int *);
    void *(*get)(Info *, const char *);
    unsigned long (*data)(Info *, void *, unsigned int, unsigned char *, unsigned long);
    unsigned long (*name)(Info *, void *, char *, unsigned long);
    int (*charset)(Info *, void *);
    void (*delete_font)(Info *, void *);
};
static const struct {const char *name, *sha256;} pins[] = {
#include "office-font-files.inc"
};
enum { FONT_COUNT = sizeof(pins)/sizeof(pins[0]), MAX_FONT = 24*1024*1024 };
typedef struct {unsigned char *bytes; size_t size; const char *name; int charset;} Font;
static Font fonts[FONT_COUNT];
static void (*add_font)(void *, const char *, int);
static unsigned long maps, reads;
static uint32_t be32(const unsigned char *p) {return (uint32_t)p[0]<<24 | (uint32_t)p[1]<<16 | (uint32_t)p[2]<<8 | p[3];}
static void enumerate(Info *s, void *mapper) {
    (void)s;
    for(int i=0;i<FONT_COUNT;i++) add_font(mapper,fonts[i].name,fonts[i].charset);
}
static void *map(Info *s,int weight,int italic,int charset,int pitch,const char *face,int *exact) {
    (void)s; (void)exact; maps++;
    int jp=charset==128 || charset==129 || charset==134 || charset==136 ||
        (face && (strcasestr(face,"japan") || strcasestr(face,"gothic") || strcasestr(face,"mincho") || strcasestr(face,"cjk")));
    int serif=(pitch&16) || (face && (strcasestr(face,"times") || strcasestr(face,"serif")));
    int index=jp ? 8+(weight>=600) : serif*4+(weight>=600)+(italic?2:0);
    fprintf(stderr,"FONT_MAP charset=%d weight=%d italic=%d face=%s bundled=%s\n",charset,weight,italic,face?face:"",fonts[index].name);
    return &fonts[index];
}
static void *get(Info *s,const char *face) {
    for(int i=0;i<FONT_COUNT;i++) if(!strcmp(face,fonts[i].name)) return &fonts[i];
    return map(s,400,0,0,0,face,NULL);
}
static unsigned long data(Info *s,void *handle,unsigned int tag,unsigned char *out,unsigned long capacity) {
    (void)s; reads++;
    Font *f=handle; size_t offset=0,size=f->size;
    if(tag) {
        size=0;
        unsigned count=(unsigned)f->bytes[4]*256+f->bytes[5];
        for(unsigned i=0;i<count;i++) {
            const unsigned char *entry=f->bytes+12+i*16;
            if(be32(entry)==tag) {offset=be32(entry+8);size=be32(entry+12);break;}
        }
    }
    if(offset>f->size || size>f->size-offset) return 0;
    if(out && capacity>=size) memcpy(out,f->bytes+offset,size);
    return size;
}
static unsigned long name(Info *s,void *handle,char *out,unsigned long capacity) {
    (void)s; Font *f=handle; size_t size=strlen(f->name)+1;
    if(out && capacity>=size) memcpy(out,f->name,size);
    return size;
}
static int charset(Info *s,void *handle) {(void)s;return ((Font *)handle)->charset;}
static void delete_font(Info *s,void *handle) {(void)s;(void)handle;}
static Info info={1,NULL,enumerate,map,get,data,name,charset,delete_font};
int vz_office_fonts(const char *library,const char *directory) {
    size_t total=0;
    for(int i=0;i<FONT_COUNT;i++) {
        char path[4096];
        if(snprintf(path,sizeof(path),"%s/%s",directory,pins[i].name)>=(int)sizeof(path)) return 1;
        FILE *file=fopen(path,"rb"); if(!file) return 2;
        if(fseek(file,0,SEEK_END)) {fclose(file);return 3;}
        long size=ftell(file);
        if(size<12 || size>MAX_FONT || total+(size_t)size>64*1024*1024) {fclose(file);return 4;}
        rewind(file); Font *f=&fonts[i]; f->bytes=malloc(size);f->size=size;
        if(!f->bytes || fread(f->bytes,1,size,file)!=(size_t)size) {fclose(file);return 5;}
        fclose(file);
        unsigned char hash[CC_SHA256_DIGEST_LENGTH];char hex[65];
        CC_SHA256(f->bytes,(CC_LONG)size,hash);
        for(int k=0;k<32;k++) snprintf(hex+k*2,3,"%02x",hash[k]);
        if(strcmp(hex,pins[i].sha256)) return 6;
        unsigned count=(unsigned)f->bytes[4]*256+f->bytes[5];
        if(12+(size_t)count*16>f->size) return 7;
        for(unsigned k=0;k<count;k++) {
            const unsigned char *entry=f->bytes+12+k*16;
            size_t offset=be32(entry+8),length=be32(entry+12);
            if(offset>f->size || length>f->size-offset) return 7;
        }
        f->name=pins[i].name;f->charset=i>=8?128:0;total+=size;
    }
    void *lib=dlopen(library,RTLD_NOW|RTLD_LOCAL);if(!lib)return 8;
    void (*set)(Info *)=dlsym(lib,"FPDF_SetSystemFontInfo");
    add_font=dlsym(lib,"FPDF_AddInstalledFont");if(!set || !add_font)return 9;
    set(&info);
    fprintf(stderr,"{\"font_bundle_preloaded\":true,\"font_files\":%d,\"font_bytes\":%zu}\n",FONT_COUNT,total);
    return 0;
}
void vz_office_font_stats(void) {
    fprintf(stderr,"{\"font_map_calls\":%lu,\"font_data_calls\":%lu,\"font_provider_path_opens_after_preload\":0}\n",maps,reads);
}
