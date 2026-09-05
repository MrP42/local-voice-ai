#include "Cancellation.hpp"
#include <whisper/whisper.h>
#include <chrono>
#include <cstring>
#include <string>
#include <memory>

static void quiet_whisper(enum ggml_log_level, const char *, void *) {}

int lv_transcribe(const char *model, const float *samples, int32_t count, char *output, int32_t capacity, LVCancellation *cancellation) {
    if (!model || !samples || count <= 0 || count > 16000 * 60 || !output || capacity < 2) return 1;
    output[0] = 0;
    whisper_log_set(quiet_whisper, nullptr);
    auto parameters = whisper_context_default_params(); parameters.use_gpu = false;
    std::unique_ptr<whisper_context, decltype(&whisper_free)> context(whisper_init_from_file_with_params(model, parameters), whisper_free);
    if (!context) return 2;
    LVDeadline deadline{cancellation, std::chrono::steady_clock::now() + std::chrono::seconds(90)};
    if (lv_should_abort(&deadline)) return 11;
    auto options = whisper_full_default_params(WHISPER_SAMPLING_GREEDY);
    options.n_threads = 4; options.language = "de"; options.translate = false; options.no_context = true;
    options.print_realtime = false; options.print_progress = false; options.print_timestamps = false; options.print_special = false;
    options.abort_callback = lv_should_abort; options.abort_callback_user_data = &deadline;
    if (whisper_full(context.get(), options, samples, count) != 0) return 3;
    std::string result;
    for (int i = 0; i < whisper_full_n_segments(context.get()); ++i) result += whisper_full_get_segment_text(context.get(), i);
    if (result.empty() || result.size() >= static_cast<size_t>(capacity)) return 4;
    std::memcpy(output, result.c_str(), result.size() + 1);
    return 0;
}
