/* VM extensions to the published ISA 0.8 launch ABI. */
#ifndef BENNETT_VM_H
#define BENNETT_VM_H
#include <bennett_launch.h>
#ifdef __cplusplus
extern "C" {
#endif
/* VM extensions: cooperative stepping and immutable diagnostic snapshots. */
typedef struct bennett_snapshot_t bennett_snapshot_t;
bennett_status bennett_run_step(bennett_program_t*, bennett_run_id);
bennett_status bennett_run_snapshot(bennett_program_t*, bennett_run_id, bennett_snapshot_t**);
const uint8_t* bennett_snapshot_jcs(const bennett_snapshot_t*, size_t* len);
void bennett_snapshot_free(bennett_snapshot_t*);
bennett_status bennett_run_reverse_to(bennett_program_t*, bennett_run_id, uint64_t history_length);
/* Last error, thread-local, valid until the next failing call on this thread. */
const char* bennett_last_error(void);
#ifdef __cplusplus
}
#endif
#endif
