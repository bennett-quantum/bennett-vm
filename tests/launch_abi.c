#include "bennett_vm.h"
#include <bennett_sched.h>
#include <assert.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
static unsigned checks;
#define EQ(a,b) do { unsigned actual=(a), expected=(b); ++checks; if(actual!=expected){fprintf(stderr,"line %d: %s got %u expected %u: %s\n",__LINE__,#a,actual,expected,bennett_last_error());abort();}}while(0)
int main(int argc, char** argv) {
    assert(argc==2);
    FILE* f=fopen(argv[1],"rb"); assert(f); fseek(f,0,SEEK_END); size_t n=(size_t)ftell(f); rewind(f);
    uint8_t* text=malloc(n); assert(fread(text,1,n,f)==n); fclose(f);
    bennett_program_t* p=NULL;
    EQ(bennett_load(text,n,NULL,0,&p),BENNETT_OK); free(text);
    bennett_args_t* a=NULL; EQ(bennett_args_new(0,&a),BENNETT_OK);
    bennett_run_id run; EQ(bennett_run(p,"host",a,NULL,0,&run),BENNETT_OK); bennett_args_free(a);
    /* §10: "bennett_unload with a run not CLOSED: E_INVALID_STATE." */
    EQ(bennett_unload(p),BENNETT_E_INVALID_STATE);
    EQ(bennett_suspend(p,run),BENNETT_OK);
    bennett_result_t* result=NULL; bennett_request_t* request=NULL;
    /* §10: "Waiting on a SUSPENDED ... run is E_INVALID_STATE." */
    EQ(bennett_wait(p,run,&result,&request),BENNETT_E_INVALID_STATE);
    EQ(bennett_resume(p,run),BENNETT_OK);
    EQ(bennett_wait(p,run,&result,&request),BENNETT_OK); assert(!result && request);
    assert(!strcmp(bennett_request_service(request),"org.bennett.test"));
    size_t len; const uint8_t* bytes=bennett_request_payload(request,&len);
    assert(len==3 && bytes[0]==165 && bytes[1]==0 && bytes[2]==0);
    const uint8_t reply[]={1,2,3,4,5}; EQ(bennett_reply(p,run,reply,5),BENNETT_OK);
    bennett_request_free(request);
    EQ(bennett_wait(p,run,&result,&request),BENNETT_OK); assert(result && !request);
    /* §10: "rlen = the full reply length (so truncation is detectable)". */
    EQ(bennett_result_nvalues(result),1); EQ(bennett_result_nfaults(result),0);
    bennett_arg_kind kind=BENNETT_ARG_F64; uint32_t width=999; uint8_t out[8]; memset(out,0xcc,8);
    /* §10: "A cap smaller ... is E_ARG and nothing is written." */
    EQ(bennett_result_get(result,0,&kind,&width,out,1),BENNETT_E_ARG);
    assert(kind==BENNETT_ARG_F64 && width==999 && out[0]==0xcc);
    EQ(bennett_result_get(result,0,&kind,&width,out,8),BENNETT_OK);
    assert(kind==BENNETT_ARG_INT && width==16 && out[0]==5 && out[1]==0);
    EQ(bennett_result_nmeasurements(result),0); EQ(bennett_result_tape_length(result),0);
    EQ(bennett_reverse(p,run),BENNETT_OK);
    EQ(bennett_wait(p,run,&result,&request),BENNETT_E_INVALID_STATE);
    EQ(bennett_run_close(p,run),BENNETT_OK); bennett_result_free(result);
    EQ(bennett_unload(p),BENNETT_OK);
    printf("C ABI: %u checks passed\n",checks); return 0;
}
