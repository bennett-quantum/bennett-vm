/* §11 published header consumer: all 15 functions, no copied declarations. */
#include <bennett_sched.h>
#include <bennett_vm.h>
#include <assert.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
static unsigned char *readfile(const char *p,size_t *n) {
 FILE *f=fopen(p,"rb"); assert(f); assert(!fseek(f,0,SEEK_END));
 long size=ftell(f);assert(size>=0);*n=(size_t)size;rewind(f);
 unsigned char *v=malloc(*n+1);assert(v);assert(fread(v,1,*n,f)==*n);v[*n]=0;fclose(f);return v;
}
int main(int argc,char **argv){
 assert(argc==4);size_t n,tlen;unsigned char *object=readfile(argv[1],&n),*target=readfile(argv[2],&tlen);
 bennett_sched_request_t *r=NULL;
 assert(bennett_sched_request_new(NULL)==BENNETT_E_ARG);
 assert(bennett_sched_request_new(&r)==BENNETT_OK&&r);
 bennett_sched_response_t *response=NULL;
 assert(bennett_sched_schedule(r,&response)==BENNETT_E_ARG&&response==NULL);
 assert(bennett_sched_request_set_object(r,object,n)==BENNETT_OK);
 assert(bennett_sched_request_set_target(r,target,tlen)==BENNETT_OK);
 assert(bennett_sched_request_set_region(r,argv[3],"forward",0)==BENNETT_OK);
 assert(bennett_sched_request_set_budget(r,3,0,0,0)==BENNETT_OK);
 assert(bennett_sched_request_set_params(r,(const uint8_t*)"{}",2)==BENNETT_OK);
 assert(bennett_sched_request_set_accuracy(r,"0")==BENNETT_OK);
 assert(bennett_sched_request_set_accuracy(r,"2")==BENNETT_E_ARG);
 assert(bennett_sched_request_set_capacity(r,"fast",1)==BENNETT_OK);
 /* §11: unsupported bound library is an explicit UNSUPPORTED response, not OK schedule. */
 assert(bennett_sched_schedule(r,&response)==BENNETT_OK&&response);
 assert(bennett_sched_response_outcome(response)==BENNETT_SCHED_UNSUPPORTED);
 assert(strstr(bennett_sched_response_reason(response),"ref@0.8"));
 size_t len=123;assert(bennett_sched_response_schedule(response,&len)==NULL&&len==0);
 len=123;assert(bennett_sched_response_certificate(response,&len)==NULL&&len==0);
 bennett_sched_response_t *second=NULL;
 assert(bennett_sched_schedule(r,&second)==BENNETT_OK);
 assert(!strcmp(bennett_sched_response_reason(response),bennett_sched_response_reason(second)));
 /* Requests own copies; response pointers remain valid after request free. */
 memset(object,0,n);memset(target,0,tlen);free(object);free(target);
 bennett_sched_request_free(r);
 assert(strstr(bennett_sched_response_reason(response),"milestone 4"));
 bennett_sched_response_free(response);bennett_sched_response_free(second);
 bennett_sched_request_free(NULL);bennett_sched_response_free(NULL);
 puts("sched ABI: all 15 published functions, explicit unsupported, copied input ownership");
 return 0;
}
