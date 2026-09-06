#include "Cancellation.hpp"
#include <llama/llama.h>
#include <chrono>
#include <cstring>
#include <memory>
#include <string>
#include <vector>
#include <mutex>

static void quiet_llama(enum ggml_log_level, const char *, void *) {}

static const char *minutes_grammar = R"GRAM(
root ::= "{" ws "\"summary\"" ws ":" ws summary "," ws "\"decisions\"" ws ":" ws indexes "," ws "\"tasks\"" ws ":" ws tasks "," ws "\"next_steps\"" ws ":" ws steps "," ws "\"follow_ups\"" ws ":" ws recommendations "," ws "\"open_questions\"" ws ":" ws indexes "}" ws
ws ::= [ \t\n\r]*
str ::= "\"" ([^"\\\x00-\x1F] | "\\" (["\\/bfnrt] | "u" [0-9a-fA-F] [0-9a-fA-F] [0-9a-fA-F] [0-9a-fA-F]))* "\"" ws
nullable ::= str | "null" ws
integer ::= ("0" | [1-9] [0-9]*) ws
summary ::= "[" ws integer ("," ws integer)? "]" ws
indexes ::= "[" ws (integer ("," ws integer)*)? "]" ws
tasks ::= "[" ws (task ("," ws task)*)? "]" ws
task ::= "{" ws "\"index\"" ws ":" ws integer "," ws "\"assignee\"" ws ":" ws nullable "," ws "\"due\"" ws ":" ws nullable "}" ws
steps ::= "[" ws (step ("," ws step)*)? "]" ws
step ::= "{" ws "\"index\"" ws ":" ws integer "," ws "\"owner\"" ws ":" ws nullable "}" ws
recommendations ::= "[" ws (recommendation ("," ws recommendation)*)? "]" ws
recommendation ::= "{" ws "\"index\"" ws ":" ws integer "," ws "\"text\"" ws ":" ws str "}" ws
)GRAM";

static int generate(const char *path, const char *prompt, char *output, int32_t capacity, LVCancellation *cancellation, bool minutes) {
    if (!path || !prompt || std::strlen(prompt) > 12000 || !output || capacity < 2) return 1;
    output[0] = 0;
    static std::once_flag initialized;
    std::call_once(initialized, [] { llama_log_set(quiet_llama, nullptr); llama_backend_init(); });
    auto mp = llama_model_default_params(); mp.n_gpu_layers = 0;
    std::unique_ptr<llama_model, decltype(&llama_model_free)> model(llama_model_load_from_file(path, mp), llama_model_free);
    if (!model) return 2;
    LVDeadline deadline{cancellation, std::chrono::steady_clock::now() + std::chrono::seconds(minutes ? 180 : 90)};
    if (lv_should_abort(&deadline)) return 11;
    auto cp = llama_context_default_params(); cp.n_ctx = minutes ? 4096 : 1024; cp.n_batch = minutes ? 3072 : 1024; cp.n_ubatch = 128;
    cp.n_threads = 4; cp.n_threads_batch = 4; cp.offload_kqv = false;
    cp.abort_callback = lv_should_abort; cp.abort_callback_data = &deadline;
    std::unique_ptr<llama_context, decltype(&llama_free)> context(llama_init_from_model(model.get(), cp), llama_free);
    if (!context) return 3;
    auto vocab = llama_model_get_vocab(model.get());
    const int prompt_limit = minutes ? 3072 : 960;
    std::vector<llama_token> tokens(prompt_limit);
    int n = llama_tokenize(vocab, prompt, static_cast<int32_t>(std::strlen(prompt)), tokens.data(), static_cast<int32_t>(tokens.size()), true, true);
    if (n <= 0 || n > prompt_limit) return 4;
    if (llama_decode(context.get(), llama_batch_get_one(tokens.data(), n)) != 0) return 5;
    std::unique_ptr<llama_sampler, decltype(&llama_sampler_free)> sampler(llama_sampler_chain_init(llama_sampler_chain_default_params()), llama_sampler_free);
    if (!sampler) return 6;
    if (minutes) {
        auto grammar = llama_sampler_init_grammar(vocab, minutes_grammar, "root");
        if (!grammar) return 6;
        llama_sampler_chain_add(sampler.get(), grammar);
    }
    llama_sampler_chain_add(sampler.get(), llama_sampler_init_greedy());
    std::string result;
    bool completed = false;
    for (int i = 0; i < (minutes ? 768 : 64); ++i) {
        if (lv_should_abort(&deadline)) return 7;
        auto token = llama_sampler_sample(sampler.get(), context.get(), -1);
        if (llama_vocab_is_eog(vocab, token)) { completed = true; break; }
        char piece[512];
        int size = llama_token_to_piece(vocab, token, piece, sizeof(piece), 0, false);
        if (size < 0 || size > static_cast<int>(sizeof(piece))) return 8;
        result.append(piece, size);
        if (result.size() >= static_cast<size_t>(capacity)) return 9;
        if (llama_decode(context.get(), llama_batch_get_one(&token, 1)) != 0) return 5;
    }
    if (result.empty() || (minutes && !completed)) return 10;
    std::memcpy(output, result.c_str(), result.size() + 1);
    return 0;
}

int lv_generate(const char *path, const char *prompt, char *output, int32_t capacity, LVCancellation *cancellation) {
    return generate(path, prompt, output, capacity, cancellation, false);
}
int lv_generate_minutes(const char *path, const char *prompt, char *output, int32_t capacity, LVCancellation *cancellation) {
    return generate(path, prompt, output, capacity, cancellation, true);
}
