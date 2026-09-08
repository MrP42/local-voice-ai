#ifndef LV_ENGINES_H
#define LV_ENGINES_H
#include <stdint.h>
#ifdef __cplusplus
extern "C" {
#endif
typedef struct LVCancellation LVCancellation;
LVCancellation *lv_cancel_create(void);
void lv_cancel_request(LVCancellation *value);
void lv_cancel_destroy(LVCancellation *value);
// Bounded, CPU-only inference. Buffers are owned by the caller; output is UTF-8.
int lv_transcribe(const char *model, const float *samples, int32_t count, char *output, int32_t capacity, LVCancellation *cancellation);
int lv_transcribe_segments(const char *model, const float *samples, int32_t count, char *output, int32_t capacity, LVCancellation *cancellation);
int lv_generate_minutes(const char *model, const char *prompt, char *output, int32_t capacity, LVCancellation *cancellation);
int lv_generate(const char *model, const char *prompt, char *output, int32_t capacity, LVCancellation *cancellation);
#ifdef __cplusplus
}
#endif
#endif
