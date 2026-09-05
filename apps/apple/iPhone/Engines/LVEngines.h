#ifndef LV_ENGINES_H
#define LV_ENGINES_H
#include <stdint.h>
#ifdef __cplusplus
extern "C" {
#endif
// Bounded, CPU-only inference. Buffers are owned by the caller; output is UTF-8.
int lv_transcribe(const char *model, const float *samples, int32_t count, char *output, int32_t capacity);
int lv_generate(const char *model, const char *prompt, char *output, int32_t capacity);
#ifdef __cplusplus
}
#endif
#endif
