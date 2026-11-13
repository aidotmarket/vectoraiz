/* Private, no-network LaunchServices launch control and registration utility.
 * Parent observes the exact executable in the process table. No file write,
 * network access or URL AppleEvent consumption is needed for launch control. */
#include <CoreServices/CoreServices.h>
#include <limits.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
int main(int argc,char **argv) {
    alarm(10);
    if(argc==3 && !strcmp(argv[1],"register")) {
        CFURLRef url=CFURLCreateFromFileSystemRepresentation(NULL,(const UInt8 *)argv[2],strlen(argv[2]),true);
        OSStatus status=LSRegisterURL(url,true);CFRelease(url);
        printf("{\"register_status\":%d}\n",(int)status);return status?1:0;
    }
    pause();return 0;
}
