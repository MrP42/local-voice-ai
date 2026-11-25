#include "Cancellation.hpp"
#include <whisper/whisper.h>
#include <chrono>
#include <cstring>
#include <string>
#include <memory>
#include <algorithm>
#include <TargetConditionals.h>
#import <Foundation/Foundation.h>

static void quiet_whisper(enum ggml_log_level, const char *, void *) {}

static void language_phase(bool detecting) {
#if DEBUG
    NSURL *documents = [[NSFileManager defaultManager]
        URLsForDirectory:NSDocumentDirectory inDomains:NSUserDomainMask].firstObject;
    NSDictionary *value = @{
        @"phase": detecting ? @"detecting_language" : @"transcribing",
        @"at": @([[NSDate date] timeIntervalSince1970])
    };
    NSData *data = [NSJSONSerialization dataWithJSONObject:value options:0 error:nil];
    [data writeToURL:[documents URLByAppendingPathComponent:@"native-phase.json"]
            options:NSDataWritingAtomic error:nil];
#else
    (void)detecting;
#endif
}

int lv_whisper_language_valid(const char *language) {
    return !language || std::strcmp(language, "auto") == 0 || whisper_lang_id(language) >= 0;
}

static int transcribe(const char *model, const float *samples, int32_t count, char *output,
                      int32_t capacity, LVCancellation *cancellation, bool segmented,
                      const char *language, const char *initial_prompt, bool allow_gpu) {
    if (!model || !samples || count <= 0 || count > 16000 * 60 || !output || capacity < 2) return 1;
    output[0] = 0;
    if (!lv_whisper_language_valid(language)) return 1;
    const auto seconds = std::max<int64_t>(90, (int64_t(count) * 3 + 15999) / 16000);
    LVDeadline deadline{cancellation, std::chrono::steady_clock::now() +
                                          std::chrono::seconds(seconds)};
    if (lv_should_abort(&deadline)) return 11;
    whisper_log_set(quiet_whisper, nullptr);
    auto parameters = whisper_context_default_params();
    parameters.use_gpu = allow_gpu && !TARGET_OS_SIMULATOR;
    parameters.flash_attn = parameters.use_gpu;
    std::unique_ptr<whisper_context, decltype(&whisper_free)> context(
        whisper_init_from_file_with_params(model, parameters), whisper_free);
    if (lv_should_abort(&deadline)) return 11;
    if (!context && parameters.use_gpu) {
        parameters.use_gpu = false;
        parameters.flash_attn = false;
        context.reset(whisper_init_from_file_with_params(model, parameters));
    }
    if (lv_should_abort(&deadline)) return 11;
    if (!context) return 2;
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
    options.abort_callback = lv_should_abort; options.abort_callback_user_data = &deadline;
    // Ein CPU-Neustart teilt sich Abbruch und ursprüngliche Deadline mit dem GPU-Lauf.
    auto infer = [&]() -> int {
        if (lv_should_abort(&deadline)) return 11;
        if (initial_prompt) {
            whisper_token tokens[224];
            const int size = whisper_tokenize(context.get(), initial_prompt, tokens, 224);
            if (size < 0 || size > 200) return 1;
        }
        options.language = language ? language : "auto";
        if (std::strcmp(options.language, "auto") == 0) {
            const int mel = whisper_pcm_to_mel(context.get(), samples, count, options.n_threads);
            if (lv_should_abort(&deadline)) return 11;
            if (mel != 0) return 3;
            // Die Erkennung selbst bietet keinen Abort-Callback; direkt davor/danach prüfen.
            language_phase(true);
            if (lv_should_abort(&deadline)) return 11;
            const int detected = whisper_lang_auto_detect(
                context.get(), 0, options.n_threads, nullptr);
            if (lv_should_abort(&deadline)) return 11;
            language_phase(false);
            if (detected < 0) return 3;
            options.language = whisper_lang_str(detected);
            if (!options.language) return 3;
        }
        if (lv_should_abort(&deadline)) return 11;
        const int status = whisper_full(context.get(), options, samples, count);
        if (lv_should_abort(&deadline)) return 11;
        return status == 0 ? 0 : 3;
    };
    int status = infer();
    if (status == 3 && parameters.use_gpu) {
        if (lv_should_abort(&deadline)) return 11;
        context.reset();
        parameters.use_gpu = false;
        parameters.flash_attn = false;
        context.reset(whisper_init_from_file_with_params(model, parameters));
        if (lv_should_abort(&deadline)) return 11;
        if (!context) return 2;
        status = infer();
    }
    if (status != 0) return status;
    std::string result;
    for (int i = 0; i < whisper_full_n_segments(context.get()); ++i) result += whisper_full_get_segment_text(context.get(), i);
    if (result.empty()) return 4;
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
        NSData *json = [NSJSONSerialization dataWithJSONObject:segments options:0 error:&error];
        if (!json || error) return 12;
        result.assign(static_cast<const char *>(json.bytes), json.length);
    }
    if (result.size() >= static_cast<size_t>(capacity)) return 12;
    std::memcpy(output, result.c_str(), result.size() + 1);
    return 0;
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
