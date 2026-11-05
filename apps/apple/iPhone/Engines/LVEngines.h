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

// Buch-Gespräch: Modell über mehrere Fragen geladen halten, exakt zählen, Token für Token streamen.
typedef struct LVLlamaModel LVLlamaModel;
// vocab_only != 0: nur Wortschatz (zum Zählen, wenige MB). Sonst volles Modell zum Erzeugen. NULL bei Fehler.
LVLlamaModel *lv_llama_load(const char *path, int32_t vocab_only);
void lv_llama_free(LVLlamaModel *model);
// Anzahl Token von `text` mit dem Tokenizer des Modells (llama_tokenize), ohne BOS. special != 0 liest
// Sondertoken-Schreibweisen als Sondertoken. -1 bei Fehler.
int32_t lv_llama_count(LVLlamaModel *model, const char *text, int32_t special);
// Wird je vollständigem UTF-8-Stück aufgerufen; Rückgabe != 0 beendet die Erzeugung ohne Fehler.
typedef int32_t (*LVTokenSink)(const char *utf8, int32_t length, void *context);
// Prompt = head + system + middle + user + tail; head/middle/tail werden mit Sondertoken tokenisiert,
// system und user nie. 0 = ok; 1 Eingabe, 2 Modell ist nur Wortschatz, 3 Kontext, 4 Prompt + max_tokens > n_ctx,
// 5 decode, 7 abgebrochen oder Zeitrahmen (120 s), 8 Token zu lang.
int lv_llama_chat(LVLlamaModel *model, const char *head, const char *system, const char *middle, const char *user,
                  const char *tail, int32_t max_tokens, int32_t n_ctx, LVTokenSink sink, void *context,
                  LVCancellation *cancellation);
#ifdef __cplusplus
}
#endif
#endif
