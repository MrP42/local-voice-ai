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
// Begrenzte Inferenz mit Metal und CPU-Rückfall. Puffer gehören dem Aufrufer; Ausgabe ist UTF-8.
int lv_transcribe(const char *model, const float *samples, int32_t count, char *output, int32_t capacity, LVCancellation *cancellation);
int lv_transcribe_segments(const char *model, const float *samples, int32_t count, char *output, int32_t capacity, LVCancellation *cancellation);
// NULL oder "auto": Laufsprache erkennen. initial_prompt: NULL oder höchstens 200 Token.
int lv_whisper_language_valid(const char *language);
// Prozessweite Freigabe; bis zur ersten aktiven App-Meldung bleibt Metal gesperrt.
void lv_set_gpu_allowed(int allowed);
typedef struct LVWhisperContext LVWhisperContext;
LVWhisperContext *lv_whisper_load(const char *model, int32_t allow_gpu, LVCancellation *cancellation);
void lv_whisper_free(LVWhisperContext *context);
int32_t lv_whisper_count(LVWhisperContext *context, const char *text);
int lv_whisper_run(LVWhisperContext *context, const float *samples, int32_t count,
                   const char *language, const char *initial_prompt, char *out_json,
                   int32_t capacity, LVCancellation *cancellation);
// guess != 0 liefert auch unter Sprachsperre unabhängige Sprach-Logits; feste Auswahl: 0.
int lv_whisper_run_with_guess(LVWhisperContext *context, const float *samples, int32_t count,
                              const char *language, const char *initial_prompt, char *out_json,
                              int32_t capacity, LVCancellation *cancellation, int32_t guess);
// allow_gpu != 0 nur bei aktivem Vordergrund; alte Aufrufe bleiben auf der CPU.
int lv_transcribe_with_options(const char *model, const float *samples, int32_t count,
                              char *output, int32_t capacity, LVCancellation *cancellation,
                              const char *language, const char *initial_prompt, int32_t allow_gpu);
int lv_transcribe_segments_with_options(const char *model, const float *samples, int32_t count,
                                       char *output, int32_t capacity,
                                       LVCancellation *cancellation, const char *language,
                                       const char *initial_prompt, int32_t allow_gpu);
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
// 5 decode, 7 abgebrochen oder Zeitrahmen (120 s), 8 Token zu lang, 12 Tokenisierung fehlgeschlagen.
// n_ctx wird auf die Trainingslänge des Modells begrenzt.
int lv_llama_chat(LVLlamaModel *model, const char *head, const char *system, const char *middle, const char *user,
                  const char *tail, int32_t max_tokens, int32_t n_ctx, LVTokenSink sink, void *context,
                  LVCancellation *cancellation);
#ifdef __cplusplus
}
#endif
#endif
