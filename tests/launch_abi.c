#include "bennett_vm.h"
#include <bennett_sched.h>
#include <assert.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
/* Deciding §10 clauses, grouped by the API entry points exercised below:
 * load/unload/budget/qreg: "Tokens are scoped to the program and destroyed at
 * unload"; "bennett_unload with a run not CLOSED: E_INVALID_STATE"; budgets
 * additionally follow §5.4, "heap_limit <= layout.heap_limit_max" and
 * "stack_limit <= 2^S_cells" (counter widths are derived from the limits).
 * args_new/set_*/
/* args_free/run: "argument i ... binds to parameter i" and "Arguments and the
 * entropy tape are copied at bennett_run". (The four kinds are tested below.)
 * wait/reply/suspend/resume: "bennett_wait blocks until the run leaves RUNNING
 * and returns OK with exactly one outcome"; "bennett_reply answers it and the
 * run is RUNNING again"; "Waiting on a SUSPENDED, REVERSED or CLOSED run is
 * E_INVALID_STATE"; the §10 state graph defines suspend/resume edges.
 * reverse/run_close: "bennett_reverse reverses post-order"; "Reverse
 * eligibility: COMPLETED; no measure"; "bennett_run_close releases the record;
 * register ids created by the run remain valid unless measured or freed".
 * result accessors: "bennett_result_get writes value i and sets *kind and
 * *width"; "Fault records are read by ... nfaults / ... fault ... in order of
 * occurrence"; "a FAILED run has at least one and exposes no values".
 * request accessors: "those bytes are copied into the request".
 * tape_open/next/free: "bennett_tape_next yields each effect in tape order";
 * "each an unsigned 64-bit little-endian field ... *alen = 8 x arity".
 * all owning objects/accessor pointers/free functions: "A function that returns
 * an object transfers ownership to the caller, released with the matching
 * *_free"; "Accessor-returned pointers are valid until the owning object is
 * freed". Estimate/coverage semantics are staged explicitly by PLAN milestone 2;
 * their exports are checked below for loud refusal and matching null-safe frees.
 */
_Static_assert(BENNETT_OK==0 && BENNETT_E_VERSION==1 && BENNETT_E_PTR_WIDTH==2 &&
 BENNETT_E_DEVICE_MISSING==3 && BENNETT_E_GATE_UNSUPPORTED==4 && BENNETT_E_TARGET==5 &&
 BENNETT_E_BUDGET==6 && BENNETT_E_QREG_NOT_CLEAN==7 && BENNETT_E_INVALID_STATE==8 &&
 BENNETT_E_NOT_REVERSIBLE==9 && BENNETT_E_SCHEDULE_INFEASIBLE==10 && BENNETT_E_ARG==11 &&
 BENNETT_E_NOMEM==12, "§10 status discriminants");
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
    EQ(bennett_run_close(p,run),BENNETT_E_INVALID_STATE);
    EQ(bennett_reply(p,run,NULL,0),BENNETT_E_INVALID_STATE);
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

    EQ(bennett_args_new(1,&a),BENNETT_OK);
    EQ(bennett_run(p,"echo",a,NULL,0,&run),BENNETT_E_ARG);
    EQ(bennett_args_set_int(a,1,9,out),BENNETT_E_ARG);bennett_args_free(a);
    /* §10: positional binding and copied buffers; all four result encodings. */
    bennett_token token; EQ(bennett_budget(p,"inc8",3,0,0,0,&token),BENNETT_OK);
    EQ(bennett_budget(p,"inc8",3,1025,0,0,&token),BENNETT_E_BUDGET);
    EQ(bennett_budget(p,"inc8",3,0,2,0,&token),BENNETT_E_BUDGET);
    /* Equal budgets do not collapse opaque token identities. */
    EQ(bennett_budget(p,"inc8",3,0,0,0,&token),BENNETT_OK);
    bennett_qreg q; EQ(bennett_qreg_new(p,"any",2,&q),BENNETT_OK);
    uint8_t bits[]={1,0};EQ(bennett_qreg_prepare(p,q,bits,2),BENNETT_OK);
    EQ(bennett_args_new(4,&a),BENNETT_OK);uint8_t integer[]={2,1};
    EQ(bennett_args_set_int(a,0,9,integer),BENNETT_OK);EQ(bennett_args_set_f64(a,1,1.0),BENNETT_OK);
    EQ(bennett_args_set_qreg(a,2,q),BENNETT_OK);EQ(bennett_args_set_token(a,3,token),BENNETT_OK);
    EQ(bennett_args_set_int(a,0,8,integer),BENNETT_OK);EQ(bennett_run(p,"echo",a,NULL,0,&run),BENNETT_E_ARG);
    EQ(bennett_args_set_int(a,0,9,integer),BENNETT_OK);
    EQ(bennett_run(p,"echo",a,NULL,0,&run),BENNETT_OK);memset(integer,0,2);bennett_args_free(a);
    EQ(bennett_qreg_free(p,q),BENNETT_E_INVALID_STATE);
    EQ(bennett_wait(p,run,&result,&request),BENNETT_OK);EQ(bennett_result_nvalues(result),4);
    EQ(bennett_result_get(result,0,&kind,&width,out,8),BENNETT_OK);assert(width==9&&kind==BENNETT_ARG_INT&&out[0]==2&&out[1]==1);
    EQ(bennett_result_get(result,1,&kind,&width,out,8),BENNETT_OK);assert(width==64&&kind==BENNETT_ARG_F64&&out[6]==0xf0&&out[7]==0x3f);
    EQ(bennett_result_get(result,2,&kind,&width,out,8),BENNETT_OK);uint64_t got=0;for(unsigned j=0;j<8;++j)got|=(uint64_t)out[j]<<(8*j);assert(got==q&&kind==BENNETT_ARG_QREG);
    EQ(bennett_result_get(result,3,&kind,&width,out,8),BENNETT_OK);got=0;for(unsigned j=0;j<8;++j)got|=(uint64_t)out[j]<<(8*j);assert(got==token&&kind==BENNETT_ARG_TOKEN);
    EQ(bennett_result_get(result,4,&kind,&width,out,8),BENNETT_E_ARG);
    EQ(bennett_run_close(p,run),BENNETT_OK);bennett_result_free(result);
    /* §10: prepare/free require a clean register; there is no reset. */
    EQ(bennett_qreg_prepare(p,q,bits,2),BENNETT_E_QREG_NOT_CLEAN);EQ(bennett_qreg_free(p,q),BENNETT_E_QREG_NOT_CLEAN);
    bennett_qreg clean;EQ(bennett_qreg_new(p,"any",1,&clean),BENNETT_OK);EQ(bennett_qreg_free(p,clean),BENNETT_OK);
    EQ(bennett_args_new(0,&a),BENNETT_OK);
    /* §7.3: five atomic effects; §10: each operand is an unsigned LE u64. */
    uint8_t entropy[]={11,22,33};EQ(bennett_run(p,"effect_run",a,entropy,3,&run),BENNETT_OK);memset(entropy,0,3);
    EQ(bennett_wait(p,run,&result,&request),BENNETT_OK);EQ(bennett_result_tape_length(result),5);
    bennett_tape_iter_t* it;EQ(bennett_tape_open(p,run,&it),BENNETT_OK);
    const char* dev;const char* op;const uint8_t* args;size_t alen;
    const char* devices[]={"entropy","console","clock","block","block"};const char* ops[]={"read","write","read","write","read"};
    for(unsigned j=0;j<5;++j){EQ(bennett_tape_next(it,&dev,&op,&args,&alen),BENNETT_OK);assert(!strcmp(dev,devices[j])&&!strcmp(op,ops[j])&&alen==(j==2?8:16));
        /* host run was reversed, so bump is again 16. */
        assert(args[0]==(j>=3?7:16));for(unsigned k=1;k<8;++k)assert(args[k]==0);
        if(j<2){assert(args[8]==3);}if(j>=3){assert(args[8]==16);}
    }
    EQ(bennett_tape_next(it,&dev,&op,&args,&alen),BENNETT_E_INVALID_STATE);
    EQ(bennett_reverse(p,run),BENNETT_OK);EQ(bennett_run_close(p,run),BENNETT_OK);bennett_result_free(result);
    /* Accessor storage belongs to iterator, survives run close and reversal. */
    assert(!strcmp(dev,"block")&&args[0]==7);bennett_tape_free(it);

    /* §10 copies entropy; each new run starts at its own tape position zero. */
    uint8_t next_entropy[]={44,55,66};
    EQ(bennett_run(p,"effect_run",a,next_entropy,3,&run),BENNETT_OK);memset(next_entropy,0,3);
    EQ(bennett_wait(p,run,&result,&request),BENNETT_OK);EQ(bennett_result_nfaults(result),0);
    bennett_snapshot_t* tape_snapshot;EQ(bennett_run_snapshot(p,run,&tape_snapshot),BENNETT_OK);
    bytes=bennett_snapshot_jcs(tape_snapshot,&len);char* tape_text=malloc(len+1);memcpy(tape_text,bytes,len);tape_text[len]=0;
    assert(strstr(tape_text,"\"bytes\":\"2c3742\""));free(tape_text);bennett_snapshot_free(tape_snapshot);
    EQ(bennett_reverse(p,run),BENNETT_OK);EQ(bennett_run_close(p,run),BENNETT_OK);bennett_result_free(result);
    /* §7.4 single-step, immutable snapshot, reverse prefix, then same forward. */
    EQ(bennett_run(p,"tree",a,NULL,0,&run),BENNETT_OK);EQ(bennett_run_step(p,run),BENNETT_OK);
    bennett_snapshot_t* snapshot;EQ(bennett_run_snapshot(p,run,&snapshot),BENNETT_OK);
    bytes=bennett_snapshot_jcs(snapshot,&len);assert(len>0);char* saved=malloc(len+1);memcpy(saved,bytes,len);saved[len]=0;assert(strstr(saved,"\"history_length\":1"));free(saved);
    EQ(bennett_suspend(p,run),BENNETT_OK);EQ(bennett_run_reverse_to(p,run,0),BENNETT_OK);EQ(bennett_resume(p,run),BENNETT_OK);
    EQ(bennett_wait(p,run,&result,&request),BENNETT_OK);EQ(bennett_reverse(p,run),BENNETT_OK);EQ(bennett_run_close(p,run),BENNETT_OK);
    assert(bytes==bennett_snapshot_jcs(snapshot,&len));bennett_snapshot_free(snapshot);bennett_result_free(result);
    /* §10 FAILED: no values, at least one ordered fault. §7.3 atomic failure. */
    EQ(bennett_run(p,"short_run",a,NULL,0,&run),BENNETT_OK);EQ(bennett_wait(p,run,&result,&request),BENNETT_OK);
    EQ(bennett_result_nvalues(result),0);EQ(bennett_result_nfaults(result),1);EQ(bennett_result_fault(result,0),10);EQ(bennett_result_tape_length(result),0);
    EQ(bennett_reverse(p,run),BENNETT_E_INVALID_STATE);
    /* Runtime extension reverses a faulted prefix, unlike whole-launch reverse. */
    EQ(bennett_run_reverse_to(p,run,0),BENNETT_OK);EQ(bennett_resume(p,run),BENNETT_OK);bennett_result_free(result);
    EQ(bennett_wait(p,run,&result,&request),BENNETT_OK);EQ(bennett_run_close(p,run),BENNETT_OK);bennett_result_free(result);
    EQ(bennett_run(p,"fault_run",a,NULL,0,&run),BENNETT_OK);EQ(bennett_wait(p,run,&result,&request),BENNETT_OK);
    EQ(bennett_result_fault(result,0),5);EQ(bennett_result_nvalues(result),0);EQ(bennett_run_close(p,run),BENNETT_OK);bennett_result_free(result);
    /* §10: run-created register IDs remain valid after close. */
    EQ(bennett_run(p,"newq",a,NULL,0,&run),BENNETT_OK);EQ(bennett_wait(p,run,&result,&request),BENNETT_OK);
    EQ(bennett_result_get(result,0,&kind,&width,out,8),BENNETT_OK);bennett_qreg created=0;for(unsigned j=0;j<8;++j)created|=(uint64_t)out[j]<<(8*j);
    EQ(bennett_run_close(p,run),BENNETT_OK);bennett_result_free(result);EQ(bennett_qreg_free(p,created),BENNETT_OK);
    bennett_args_free(a);
    /* §5.4: B=0 and B=2 fault 1; B=3 commits inc8(0)=1. */
    for(unsigned budget=0;budget<=3;++budget){if(budget==1)continue;
        EQ(bennett_budget(p,"inc8",budget,0,0,0,&token),BENNETT_OK);EQ(bennett_args_new(1,&a),BENNETT_OK);EQ(bennett_args_set_token(a,0,token),BENNETT_OK);
        EQ(bennett_run(p,"oracle_run",a,NULL,0,&run),BENNETT_OK);bennett_args_free(a);EQ(bennett_wait(p,run,&result,&request),BENNETT_OK);
        EQ(bennett_result_get(result,0,&kind,&width,out,8),BENNETT_OK);assert(out[0]==(budget==3?0:1));
        EQ(bennett_result_get(result,1,&kind,&width,out,8),BENNETT_OK);assert(out[0]==(budget==3?1:0));
        EQ(bennett_result_nmeasurements(result),3);bennett_qreg measured;size_t nbits;uint8_t mbits[8];
        EQ(bennett_result_measurement(result,1,&measured,mbits,0,&nbits),BENNETT_E_ARG);
        EQ(bennett_result_measurement(result,1,&measured,mbits,8,&nbits),BENNETT_OK);assert(nbits==8&&mbits[0]==(budget==3?1:0));
        EQ(bennett_reverse(p,run),BENNETT_E_NOT_REVERSIBLE);EQ(bennett_run_close(p,run),BENNETT_OK);bennett_result_free(result);
    }

    /* §10: independent registers can run concurrently; undo touches its own deltas. */
    bennett_qreg qa,qb;EQ(bennett_qreg_new(p,"any",1,&qa),BENNETT_OK);EQ(bennett_qreg_new(p,"any",1,&qb),BENNETT_OK);
    bennett_run_id ra,rb;EQ(bennett_args_new(1,&a),BENNETT_OK);EQ(bennett_args_set_qreg(a,0,qa),BENNETT_OK);
    EQ(bennett_run(p,"mutate",a,NULL,0,&ra),BENNETT_OK);EQ(bennett_run(p,"mutate",a,NULL,0,&rb),BENNETT_E_INVALID_STATE);
    EQ(bennett_args_set_qreg(a,0,qb),BENNETT_OK);EQ(bennett_run(p,"mutate",a,NULL,0,&rb),BENNETT_OK);
    EQ(bennett_wait(p,ra,&result,&request),BENNETT_OK);bennett_result_free(result);
    EQ(bennett_wait(p,rb,&result,&request),BENNETT_OK);bennett_result_free(result);
    EQ(bennett_reverse(p,ra),BENNETT_OK);EQ(bennett_run_close(p,ra),BENNETT_OK);EQ(bennett_qreg_free(p,qa),BENNETT_OK);
    EQ(bennett_run_close(p,rb),BENNETT_OK);EQ(bennett_run(p,"readq",a,NULL,0,&rb),BENNETT_OK);
    EQ(bennett_wait(p,rb,&result,&request),BENNETT_OK);EQ(bennett_result_get(result,0,&kind,&width,out,8),BENNETT_OK);assert(out[0]==1);
    EQ(bennett_run_close(p,rb),BENNETT_OK);bennett_result_free(result);bennett_args_free(a);
    /* §10 stubs permitted by PLAN milestone 2: loud failure, no fake estimate. */
    bennett_estimate_t* estimate=(void*)1;EQ(bennett_estimate_run(p,"inc8",token,&estimate),BENNETT_E_TARGET);assert(!estimate&&strstr(bennett_last_error(),"unimplemented:"));
    EQ(bennett_estimate_nresources(NULL),0);EQ(bennett_estimate_get(NULL,0,NULL,NULL),BENNETT_E_TARGET);assert(!bennett_estimate_certificate(NULL,&len)&&len==0);bennett_estimate_free(NULL);
    bennett_coverage_t* coverage=(void*)1;EQ(bennett_coverage_get(p,&coverage),BENNETT_E_TARGET);assert(!coverage&&strstr(bennett_last_error(),"unimplemented:"));assert(!bennett_coverage_jcs(NULL,&len)&&len==0);bennett_coverage_free(NULL);
    EQ(bennett_unload(p),BENNETT_OK);
    printf("C ABI: %u checks passed\n",checks); return 0;
}
