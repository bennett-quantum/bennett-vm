/* Independently selected published 0.9 declarations and provider. */
#define BENNETT_ABI_FAMILY 9
#include "bennett_vm.h"
#include <bennett_sched_0_9.h>
#include <bennett_estimate_accuracy_1.h>
#include <assert.h>
#include <stdio.h>
#include <string.h>
#include <stdlib.h>
int main(int argc, char** argv) {
    assert(argc == 3);
    FILE* file = fopen(argv[1], "rb"); assert(file);
    assert(fseek(file, 0, SEEK_END) == 0);
    size_t size = (size_t)ftell(file); rewind(file);
    uint8_t* object = malloc(size); assert(object);
    assert(fread(object, 1, size, file) == size); fclose(file);
    bennett_program_t* program = NULL;
    assert(bennett_load(object, size, NULL, 0, &program) == BENNETT_OK);
    free(object);
    bennett_token token;
    assert(bennett_budget(program, "inc8", 3, 0, 0, 0, &token) == BENNETT_OK);
    bennett_estimate_t* rejected = (void*)1;
    assert(bennett_estimate_run(program, "inc8", token, "forward", 0,
        (const uint8_t*)"[]", 2, &rejected) == BENNETT_E_VERSION);
    assert(rejected == NULL);
    assert(bennett_unload(program) == BENNETT_OK);
    const uint8_t* bytes = NULL;
    size_t len = 0;
    const char* launch = "{\"abi\":\"launch/0.9\",\"features\":[],\"schema\":\"bennett.abi-features/1\"}";
    const char* sched = "{\"abi\":\"sched/0.9\",\"features\":[],\"schema\":\"bennett.abi-features/1\"}";
    assert(bennett_launch_features(&bytes, &len) == BENNETT_OK);
    assert(len == strlen(launch) && memcmp(bytes, launch, len) == 0);
    assert(bennett_sched_features(&bytes, &len) == BENNETT_OK);
    assert(len == strlen(sched) && memcmp(bytes, sched, len) == 0);
    bennett_estimate_t* estimate = (void*)1;
    assert(bennett_estimate_run(NULL, "p", 0, "forward", 0,
        (const uint8_t*)"[]", 2, &estimate) == BENNETT_E_ARG);
    assert(estimate == NULL);
    estimate = (void*)1;
    assert(bennett_estimate_run_with_token_aliases(NULL, "p", 0, "forward", 0,
        (const uint8_t*)"[]", 2, (const uint8_t*)"[]", 2, &estimate) == BENNETT_E_ARG);
    assert(estimate == NULL);
    estimate = (void*)1;
    assert(bennett_estimate_run_at_accuracy(NULL, "p", 0, "forward", 0,
        (const uint8_t*)"[]", 2, (const uint8_t*)"[]", 2, "0.1", &estimate) == BENNETT_E_ARG);
    assert(estimate == NULL);
    estimate = (void*)1;
    assert(bennett_estimate_run_at_accuracy(NULL, "p", 0, "forward", 0,
        (const uint8_t*)"[]", 2, (const uint8_t*)"[]", 2, NULL, &estimate) == BENNETT_E_ARG);
    assert(estimate == NULL);
    file = fopen(argv[2], "rb"); assert(file);
    assert(fseek(file, 0, SEEK_END) == 0);
    size = (size_t)ftell(file); rewind(file);
    object = malloc(size); assert(object);
    assert(fread(object, 1, size, file) == size); fclose(file);
    program = NULL;
    assert(bennett_load(object, size, NULL, 0, &program) == BENNETT_OK);
    free(object);
    assert(bennett_budget(program, "p", 1, 0, 0, 0, &token) == BENNETT_OK);
    const char* missing = "E_ESTIMATE_ACCURACY_REQUIRED: use bennett_estimate_run_at_accuracy with epsilon_decimal";
    estimate = (void*)1;
    assert(bennett_estimate_run(program, "p", token, "forward", 0,
        (const uint8_t*)"[]", 2, &estimate) == BENNETT_E_ARG);
    assert(estimate == NULL && strcmp(bennett_last_error(), missing) == 0);
    estimate = (void*)1;
    assert(bennett_estimate_run_with_token_aliases(program, "p", token, "forward", 0,
        (const uint8_t*)"[]", 2, (const uint8_t*)"[]", 2, &estimate) == BENNETT_E_ARG);
    assert(estimate == NULL && strcmp(bennett_last_error(), missing) == 0);
    estimate = (void*)1;
    assert(bennett_estimate_run_at_accuracy(program, "p", token, "forward", 0,
        (const uint8_t*)"[]", 2, (const uint8_t*)"[]", 2, "0.1", &estimate) == BENNETT_E_TARGET);
    assert(estimate == NULL);
    assert(bennett_unload(program) == BENNETT_OK);
    puts("0.9 published headers, queries and estimate dispatch: PASS");
}
