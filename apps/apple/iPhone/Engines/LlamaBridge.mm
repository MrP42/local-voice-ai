#include "LVEngines.h"
#include <llama/llama.h>
#include <chrono>
#include <cstring>
#include <memory>
#include <string>
#include <vector>
#include <mutex>

static void quiet_llama(enum ggml_log_level, const char *, void *) {}
static bool llama_deadline(void *value) {
    return std::chrono::steady_clock::now() > *static_cast<std::chrono::steady_clock::time_point *>(value);
}
int lv_generate(const char *path, const char *prompt, char *output, int32_t capacity) {
    if (!path || !prompt || std::strlen(prompt) > 12000 || !output || capacity < 2) return 1;
    output[0] = 0;
    static std::once_flag initialized;
    std::call_once(initialized, [] { llama_log_set(quiet_llama, nullptr); llama_backend_init(); });
    auto mp = llama_model_default_params(); mp.n_gpu_layers = 0;
    std::unique_ptr<llama_model, decltype(&llama_model_free)> model(llama_model_load_from_file(path, mp), llama_model_free);
    if (!model) return 2;
    auto deadline = std::chrono::steady_clock::now() + std::chrono::seconds(90);
    auto cp = llama_context_default_params(); cp.n_ctx = 1024; cp.n_batch = 1024; cp.n_ubatch = 128;
    cp.n_threads = 4; cp.n_threads_batch = 4; cp.offload_kqv = false;
    cp.abort_callback = llama_deadline; cp.abort_callback_data = &deadline;
    std::unique_ptr<llama_context, decltype(&llama_free)> context(llama_init_from_model(model.get(), cp), llama_free);
    if (!context) return 3;
    auto vocab = llama_model_get_vocab(model.get());
    std::vector<llama_token> tokens(960);
    int n = llama_tokenize(vocab, prompt, static_cast<int32_t>(std::strlen(prompt)), tokens.data(), static_cast<int32_t>(tokens.size()), true, true);
    if (n <= 0 || n > 960) return 4;
    if (llama_decode(context.get(), llama_batch_get_one(tokens.data(), n)) != 0) return 5;
    std::unique_ptr<llama_sampler, decltype(&llama_sampler_free)> sampler(llama_sampler_init_greedy(), llama_sampler_free);
    if (!sampler) return 6;
    std::string result;
    for (int i = 0; i < 64; ++i) {
        if (llama_deadline(&deadline)) return 7;
        auto token = llama_sampler_sample(sampler.get(), context.get(), -1);
        if (llama_vocab_is_eog(vocab, token)) break;
        char piece[512];
        int size = llama_token_to_piece(vocab, token, piece, sizeof(piece), 0, false);
        if (size < 0 || size > static_cast<int>(sizeof(piece))) return 8;
        result.append(piece, size);
        if (result.size() >= static_cast<size_t>(capacity)) return 9;
        if (llama_decode(context.get(), llama_batch_get_one(&token, 1)) != 0) return 5;
    }
    if (result.empty()) return 10;
    std::memcpy(output, result.c_str(), result.size() + 1);
    return 0;
}
