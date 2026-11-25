#include "Cancellation.hpp"
#include <whisper/whisper.h>
#include <chrono>
#include <cstring>
#include <string>
#include <memory>
#include <algorithm>
#include <cmath>
#include <limits>
#include <mutex>
#include <TargetConditionals.h>
#import <Foundation/Foundation.h>

static void quiet_whisper(enum ggml_log_level, const char *, void *) {}

// Bit 0 erlaubt Metal; jeder Entzug erhöht die Generation, auch bei schnellem Reaktivieren.
static std::atomic<uint64_t> gpu_permission{0};

// Alle Einstiege, auch der Abschluss-Pass, teilen sich diese Sperre.
static std::timed_mutex whisper_mutex;
static bool acquire_whisper(std::unique_lock<std::timed_mutex> &lock, LVDeadline *deadline) {
    while (!lock.try_lock_for(std::chrono::milliseconds(20))) {
        if (lv_should_abort(deadline)) return false;
    }
    return !lv_should_abort(deadline);
}

struct LVWhisperContext {
    std::string model;
    whisper_context_params parameters;
    uint64_t permission;
    std::unique_ptr<whisper_context, decltype(&whisper_free)> context{nullptr, whisper_free};
};

void lv_set_gpu_allowed(int allowed) {
    auto previous = gpu_permission.load();
    uint64_t next;
    do {
        next = allowed ? previous | 1 : (previous + 2) & ~uint64_t(1);
    } while (!gpu_permission.compare_exchange_weak(previous, next));
}

struct LVWhisperRun {
    LVDeadline *deadline;
    bool use_gpu;
    uint64_t permission;
};

static int whisper_stop(LVWhisperRun *run) {
    if (lv_should_abort(run->deadline)) return 11;
    // Eigener Grund für CPU-Rückfall; eine neue Freigabe rettet diesen GPU-Lauf nicht.
    if (run->use_gpu && gpu_permission.load() != run->permission) return 13;
    return 0;
}

static bool whisper_should_abort(void *value) {
    return whisper_stop(static_cast<LVWhisperRun *>(value)) != 0;
}

static void phase_marker(const char *phase) {
#if DEBUG
    NSURL *documents = [[NSFileManager defaultManager]
        URLsForDirectory:NSDocumentDirectory inDomains:NSUserDomainMask].firstObject;
    NSDictionary *value = @{
        @"phase": [NSString stringWithUTF8String:phase],
        @"at": @([[NSDate date] timeIntervalSince1970])
    };
    NSData *data = [NSJSONSerialization dataWithJSONObject:value options:0 error:nil];
    [data writeToURL:[documents URLByAppendingPathComponent:@"native-phase.json"]
            options:NSDataWritingAtomic error:nil];
#else
    (void)phase;
#endif
}

static void language_phase(bool detecting) {
    phase_marker(detecting ? "detecting_language" : "preparing");
}

struct LVPhaseEnd {
    ~LVPhaseEnd() { phase_marker("idle"); }
};

int lv_whisper_language_valid(const char *language) {
    return !language || std::strcmp(language, "auto") == 0 || whisper_lang_id(language) >= 0;
}

static int transcribe(const char *model, const float *samples, int32_t count, char *output,
                      int32_t capacity, LVCancellation *cancellation, bool segmented,
                      const char *language, const char *initial_prompt, bool allow_gpu,
                      LVWhisperContext *held = nullptr, bool guess = false) {
    if (!model || !samples || count <= 0 || count > 16000 * 60 || !output || capacity < 2) return 1;
    output[0] = 0;
    if (!lv_whisper_language_valid(language)) return 1;
    LVInferenceLease inference(cancellation);
    if (!inference.acquired) return 11;
    LVDeadline deadline{cancellation, std::chrono::steady_clock::time_point::max()};
    std::unique_lock<std::timed_mutex> lock(whisper_mutex, std::defer_lock);
    if (!acquire_whisper(lock, &deadline)) return 11;
    LVPhaseEnd phase_end;
    if (lv_should_abort(&deadline)) return 11;
    whisper_log_set(quiet_whisper, nullptr);
    LVWhisperContext temporary;
    auto &parameters = held ? held->parameters : temporary.parameters;
    const auto permission = held ? held->permission : gpu_permission.load();
    if (!held) {
        parameters = whisper_context_default_params();
        parameters.use_gpu = allow_gpu && !TARGET_OS_SIMULATOR && !TARGET_OS_WATCH && (permission & 1) != 0;
        parameters.flash_attn = parameters.use_gpu;
    }
    LVWhisperRun run{&deadline, parameters.use_gpu, permission};
    auto &context = held ? held->context : temporary.context;
    auto load_cpu = [&]() {
        context.reset();
        run.use_gpu = false;
        parameters.use_gpu = false;
        parameters.flash_attn = false;
        phase_marker("loading");
        context.reset(whisper_init_from_file_with_params(model, parameters));
    };
    auto load = [&]() -> int {
        if (lv_should_abort(&deadline)) return 11;
        // Modellladen hat keinen Abbruchcallback; vor und nach dem Aufruf prüfen.
        if (whisper_stop(&run) == 13) {
            load_cpu();
        } else {
            if (!context) {
                phase_marker("loading");
                context.reset(whisper_init_from_file_with_params(model, parameters));
            }
            if (lv_should_abort(&deadline)) return 11;
            if (parameters.use_gpu && (!context || whisper_stop(&run) == 13)) load_cpu();
        }
        return lv_should_abort(&deadline) ? 11 : 0;
    };
    if (const int stop = load()) return stop;
    if (!context) return 2;
    // Beauftragt: dreifache Audiodauer, mindestens 90 s, erst nach dem Modellladen.
    const auto seconds = std::max<int64_t>(90, (int64_t(count) * 3 + 15999) / 16000);
    deadline.deadline = std::chrono::steady_clock::now() + std::chrono::seconds(seconds);
    const char *name = std::strrchr(model, '/');
    name = name ? name + 1 : model;
    const bool turbo = std::strcmp(name, "ggml-large-v3-turbo-q5_0.bin") == 0;
    auto options = whisper_full_default_params(segmented && turbo
        ? WHISPER_SAMPLING_BEAM_SEARCH : WHISPER_SAMPLING_GREEDY);
    if (segmented && turbo) options.beam_search.beam_size = 3;
    options.n_threads = 4;
    options.language = language ? language : "auto";
    options.detect_language = false;
    options.initial_prompt = initial_prompt;
    options.translate = false; options.no_context = true;
    options.print_realtime = false; options.print_progress = false; options.print_timestamps = false; options.print_special = false;
    options.abort_callback = whisper_should_abort; options.abort_callback_user_data = &run;
    std::string heard;
    double probability = 0;
    // Ein CPU-Neustart teilt sich Abbruch und ursprüngliche Deadline mit dem GPU-Lauf.
    auto infer = [&]() -> int {
        if (const int stop = whisper_stop(&run)) return stop;
        options.language = language ? language : "auto";
        if (guess || std::strcmp(options.language, "auto") == 0) {
            if (!whisper_is_multilingual(context.get())) {
                heard = "en"; probability = 1;
                if (std::strcmp(options.language, "auto") == 0) options.language = "en";
            } else {
                language_phase(true);
                auto detection = whisper_full_default_params(WHISPER_SAMPLING_GREEDY);
                detection.n_threads = options.n_threads;
                detection.duration_ms = 30000;
                detection.single_segment = true;
                detection.no_timestamps = true;
                detection.max_tokens = 1;
                detection.temperature_inc = 0;
                detection.n_max_text_ctx = 0;
                detection.language = "en";
                detection.detect_language = false;
                detection.translate = false;
                detection.no_context = true;
                detection.print_realtime = false; detection.print_progress = false;
                detection.print_timestamps = false; detection.print_special = false;
                detection.abort_callback = whisper_should_abort;
                detection.abort_callback_user_data = &run;
                if (const int stop = whisper_stop(&run)) return stop;
                const int encoded = whisper_full(context.get(), detection, samples,
                                                 std::min<int32_t>(count, 16000 * 30));
                if (const int stop = whisper_stop(&run)) return stop;
                if (encoded != 0) return 3;
                // 1.7.5 behält kv_cross im selben Default-State für whisper_decode.
                // n_past=0 ersetzt den Textkontext; SOT liefert die Sprach-Logits.
                const whisper_token sot = whisper_token_sot(context.get());
                if (const int stop = whisper_stop(&run)) return stop;
                const int decoded = whisper_decode(context.get(), &sot, 1, 0, options.n_threads);
                if (const int stop = whisper_stop(&run)) return stop;
                if (decoded != 0) return 3;
                const float *logits = whisper_get_logits(context.get());
                if (!logits) return 3;
                int detected = -1;
                float best = -std::numeric_limits<float>::infinity();
                for (int id = 0; id <= whisper_lang_max_id(); ++id) {
                    const float score = logits[whisper_token_lang(context.get(), id)];
                    if (std::isfinite(score) && score > best) { best = score; detected = id; }
                }
                if (const int stop = whisper_stop(&run)) return stop;
                language_phase(false);
                if (detected < 0) return 3;
                options.language = whisper_lang_str(detected);
                if (!options.language) return 3;
                heard = options.language;
                double sum = 0;
                for (int id = 0; id <= whisper_lang_max_id(); ++id) {
                    const float score = logits[whisper_token_lang(context.get(), id)];
                    if (std::isfinite(score)) sum += std::exp(double(score - best));
                }
                probability = sum > 0 ? 1.0 / sum : 0;
                // Erkennung unabhängig von der gewünschten Lesesprache.
                if (language && std::strcmp(language, "auto") != 0) options.language = language;
            }
        }
        if (const int stop = whisper_stop(&run)) return stop;
        if (initial_prompt) {
            whisper_token tokens[224];
            const int size = whisper_tokenize(context.get(), initial_prompt, tokens, 224);
            if (size < 0 || size > 200) return 1;
        }
        if (const int stop = whisper_stop(&run)) return stop;
        phase_marker("transcribing");
        const int status = whisper_full(context.get(), options, samples, count);
        if (const int stop = whisper_stop(&run)) return stop;
        return status == 0 ? 0 : 3;
    };
    int status = infer();
    if ((status == 3 || status == 13) && parameters.use_gpu) {
        if (lv_should_abort(&deadline)) return 11;
        load_cpu();
        if (lv_should_abort(&deadline)) return 11;
        if (!context) return 2;
        status = infer();
    }
    if (status != 0) return status;
    std::string result;
    for (int i = 0; i < whisper_full_n_segments(context.get()); ++i) result += whisper_full_get_segment_text(context.get(), i);
    if (result.empty() && !held) return 4;
    if (segmented) {
        NSMutableArray *segments = [NSMutableArray array];
        // Whisper erkennt einmal je Lauf; MeetingLibrary übergibt höchstens 30 Sekunden.
        const char *code = whisper_lang_str(whisper_full_lang_id(context.get()));
        NSString *detected = code ? [NSString stringWithUTF8String:code] : nil;
        if (!detected) return 12;
        for (int i = 0; i < whisper_full_n_segments(context.get()); ++i) {
            double start = std::max(0.0, whisper_full_get_segment_t0(context.get(), i) / 100.0);
            double end = std::min(count / 16000.0, whisper_full_get_segment_t1(context.get(), i) / 100.0);
            NSString *text = [NSString stringWithUTF8String:whisper_full_get_segment_text(context.get(), i)];
            if (!text || start > end) return 12;
            [segments addObject:@{@"index": @(i), @"start": @(start), @"end": @(end),
                                  @"text": text, @"language": detected}];
        }
        NSError *error = nil;
        id body = segments;
        if (held) {
            NSString *language = heard.empty() ? detected : [NSString stringWithUTF8String:heard.c_str()];
            body = @{@"lines": segments, @"language": language, @"prob": @(probability)};
        }
        NSData *json = [NSJSONSerialization dataWithJSONObject:body options:0 error:&error];
        if (!json || error) return 12;
        result.assign(static_cast<const char *>(json.bytes), json.length);
    }
    if (result.size() >= static_cast<size_t>(capacity)) return 12;
    std::memcpy(output, result.c_str(), result.size() + 1);
    return 0;
}

LVWhisperContext *lv_whisper_load(const char *model, int32_t allow_gpu, LVCancellation *cancellation) {
    if (!model) return nullptr;
    LVInferenceLease inference(cancellation);
    if (!inference.acquired) return nullptr;
    LVDeadline deadline{cancellation, std::chrono::steady_clock::time_point::max()};
    std::unique_lock<std::timed_mutex> lock(whisper_mutex, std::defer_lock);
    if (!acquire_whisper(lock, &deadline)) return nullptr;
    LVPhaseEnd phase_end;
    whisper_log_set(quiet_whisper, nullptr);
    auto held = std::make_unique<LVWhisperContext>();
    held->model = model;
    held->permission = gpu_permission.load();
    held->parameters = whisper_context_default_params();
    held->parameters.use_gpu = allow_gpu && !TARGET_OS_SIMULATOR && !TARGET_OS_WATCH && (held->permission & 1) != 0;
    held->parameters.flash_attn = held->parameters.use_gpu;
    phase_marker("loading");
    held->context.reset(whisper_init_from_file_with_params(model, held->parameters));
    if (lv_should_abort(&deadline)) return nullptr;
    if (held->parameters.use_gpu && (!held->context || gpu_permission.load() != held->permission)) {
        held->context.reset();
        held->parameters.use_gpu = false; held->parameters.flash_attn = false;
        phase_marker("loading");
        held->context.reset(whisper_init_from_file_with_params(model, held->parameters));
    }
    if (!held->context || lv_should_abort(&deadline)) return nullptr;
    return held.release();
}

void lv_whisper_free(LVWhisperContext *context) {
    LVInferenceLease inference;
    std::lock_guard<std::timed_mutex> lock(whisper_mutex);
    delete context;
}

int32_t lv_whisper_count(LVWhisperContext *held, const char *text) {
    if (!held || !text) return -1;
    LVInferenceLease inference;
    std::lock_guard<std::timed_mutex> lock(whisper_mutex);
    whisper_token tokens[224];
    return whisper_tokenize(held->context.get(), text, tokens, 224);
}

int lv_whisper_run_with_guess(LVWhisperContext *held, const float *samples, int32_t count,
                              const char *language, const char *initial_prompt, char *output,
                              int32_t capacity, LVCancellation *cancellation, int32_t guess) {
    if (!held) return 1;
    return transcribe(held->model.c_str(), samples, count, output, capacity, cancellation, true,
                      language, initial_prompt, false, held, guess != 0);
}

int lv_whisper_run(LVWhisperContext *held, const float *samples, int32_t count,
                   const char *language, const char *initial_prompt, char *output,
                   int32_t capacity, LVCancellation *cancellation) {
    return lv_whisper_run_with_guess(held, samples, count, language, initial_prompt, output,
                                     capacity, cancellation, 1);
}

int lv_transcribe(const char *model, const float *samples, int32_t count, char *output, int32_t capacity, LVCancellation *cancellation) {
    return lv_transcribe_with_options(model, samples, count, output, capacity, cancellation,
                                      "auto", nullptr, 0);
}
int lv_transcribe_segments(const char *model, const float *samples, int32_t count, char *output, int32_t capacity, LVCancellation *cancellation) {
    return lv_transcribe_segments_with_options(model, samples, count, output, capacity,
                                               cancellation, "auto", nullptr, 0);
}

int lv_transcribe_with_options(const char *model, const float *samples, int32_t count,
                              char *output, int32_t capacity, LVCancellation *cancellation,
                              const char *language, const char *initial_prompt, int32_t allow_gpu) {
    return transcribe(model, samples, count, output, capacity, cancellation, false,
                      language, initial_prompt, allow_gpu != 0);
}

int lv_transcribe_segments_with_options(const char *model, const float *samples, int32_t count,
                                       char *output, int32_t capacity,
                                       LVCancellation *cancellation, const char *language,
                                       const char *initial_prompt, int32_t allow_gpu) {
    return transcribe(model, samples, count, output, capacity, cancellation, true,
                      language, initial_prompt, allow_gpu != 0);
}
