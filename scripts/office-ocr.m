/* Parent-launched S1934 OCR executable. No parser spawn or runtime compiler. */
#import <Foundation/Foundation.h>
#import <Vision/Vision.h>
extern int vz_office_bootstrap(const char *);
extern int vz_office_probe_main(int,char **);
extern int vz_office_memory_probe(const char *,const char *);
int main(int argc,char **argv) {
    @autoreleasepool {
        if(argc<3) return 64;
        NSError *error=nil;
        NSString *profile=[NSString stringWithContentsOfFile:[NSString stringWithUTF8String:argv[1]] encoding:NSUTF8StringEncoding error:&error];
        if(!profile) return 65;
        /* Initialize role runtime before counting its address-space baseline. */
        VNRecognizeTextRequest *request=[VNRecognizeTextRequest new];
        request.recognitionLevel=VNRequestTextRecognitionLevelAccurate;
        request.usesLanguageCorrection=YES;
        int status=vz_office_bootstrap(profile.UTF8String);
        if(status) return status;
        if(!strcmp(argv[2],"probe")) {if(argc!=7) return 64;return vz_office_probe_main(5,argv+2);}
        if(!strcmp(argv[2],"memory")) {if(argc!=5) return 64;return vz_office_memory_probe(argv[3],argv[4]);}
        fprintf(stderr,"{\"ready\":true,\"role\":\"ocr\"}\n");fflush(stderr);
        NSData *data=[[NSFileHandle fileHandleWithStandardInput] readDataUpToLength:16*1024*1024+1 error:&error];
        if(!data || data.length>16*1024*1024) return 66;
        VNImageRequestHandler *handler=[[VNImageRequestHandler alloc] initWithData:data options:@{}];
        if(![handler performRequests:@[request] error:&error]) {fprintf(stderr,"Vision refused: %s\n",error.description.UTF8String);return 67;}
        NSMutableArray *lines=[NSMutableArray new];
        for(VNRecognizedTextObservation *observation in request.results) {
            VNRecognizedText *text=[[observation topCandidates:1] firstObject];
            if(text) [lines addObject:text.string];
        }
        NSData *result=[NSJSONSerialization dataWithJSONObject:@{ @"lines":lines } options:0 error:&error];
        if(!result || result.length>1024*1024) return 68;
        [[NSFileHandle fileHandleWithStandardOutput] writeData:result];
    }
    return 0;
}
