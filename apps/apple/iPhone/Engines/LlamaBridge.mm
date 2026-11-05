#include "Cancellation.hpp"
#include <llama/llama.h>
#include <chrono>
#include <cstring>
#include <memory>
#include <string>
#include <vector>
#include <mutex>
#include <algorithm>
#include <TargetConditionals.h>

static void quiet_llama(enum ggml_log_level, const char *, void *) {}

static void ensure_backend() {
    static std::once_flag initialized;
    std::call_once(initialized, [] { llama_log_set(quiet_llama, nullptr); llama_backend_init(); });
}

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
    ensure_backend();
    auto mp = llama_model_default_params(); mp.n_gpu_layers = 0;
    std::unique_ptr<llama_model, decltype(&llama_model_free)> model(llama_model_load_from_file(path, mp), llama_model_free);
    if (!model) return 2;
    LVDeadline deadline{cancellation, std::chrono::steady_clock::now() + std::chrono::seconds(minutes ? 180 : 90)};
    if (lv_should_abort(&deadline)) return 11;
    auto cp = llama_context_default_params(); cp.n_ctx = 4096; cp.n_batch = 3072; cp.n_ubatch = 128;
    cp.n_threads = 4; cp.n_threads_batch = 4; cp.offload_kqv = false;
    cp.abort_callback = lv_should_abort; cp.abort_callback_data = &deadline;
    std::unique_ptr<llama_context, decltype(&llama_free)> context(llama_init_from_model(model.get(), cp), llama_free);
    if (!context) return 3;
    auto vocab = llama_model_get_vocab(model.get());
    const int prompt_limit = 3072;
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

// MARK: Buch-Gespräch (Epic #307, G6)

struct LVLlamaModel {
    llama_model *model;
    bool vocab_only;
};

LVLlamaModel *lv_llama_load(const char *path, int32_t vocab_only) {
    if (!path) return nullptr;
    ensure_backend();
    auto mp = llama_model_default_params();
    mp.vocab_only = vocab_only != 0;
#if TARGET_OS_SIMULATOR
    mp.n_gpu_layers = 0;
#else
    // Das Gespräch läuft nur im Vordergrund (Plan §8.3), daher darf die GPU rechnen.
    mp.n_gpu_layers = vocab_only ? 0 : 99;
#endif
    llama_model *model = llama_model_load_from_file(path, mp);
    if (!model) return nullptr;
    return new LVLlamaModel{model, vocab_only != 0};
}

void lv_llama_free(LVLlamaModel *value) {
    if (!value) return;
    llama_model_free(value->model);
    delete value;
}

static bool append_tokens(const llama_vocab *vocab, const char *text, bool special, std::vector<llama_token> &out) {
    const int32_t length = static_cast<int32_t>(std::strlen(text));
    if (length == 0) return true;
    std::vector<llama_token> piece(static_cast<size_t>(length) + 8);
    int32_t n = llama_tokenize(vocab, text, length, piece.data(), static_cast<int32_t>(piece.size()), false, special);
    if (n < 0) {
        piece.resize(static_cast<size_t>(-n));
        n = llama_tokenize(vocab, text, length, piece.data(), static_cast<int32_t>(piece.size()), false, special);
        if (n < 0) return false;
    }
    out.insert(out.end(), piece.begin(), piece.begin() + n);
    return true;
}

int32_t lv_llama_count(LVLlamaModel *value, const char *text, int32_t special) {
    if (!value || !text) return -1;
    if (std::strlen(text) == 0) return 0;
    auto vocab = llama_model_get_vocab(value->model);
    std::vector<llama_token> tokens;
    if (!append_tokens(vocab, text, special != 0, tokens)) return -1;
    return static_cast<int32_t>(tokens.size());
}

// Länge des längsten Präfixes, das mit einem vollständigen UTF-8-Zeichen endet.
static size_t utf8_complete_prefix(const std::string &bytes) {
    size_t end = bytes.size();
    size_t back = 0;
    while (back < 4 && back < end) {
        const unsigned char c = static_cast<unsigned char>(bytes[end - 1 - back]);
        if ((c & 0xC0) == 0x80) { ++back; continue; }
        size_t need = (c & 0x80) == 0 ? 1 : (c & 0xE0) == 0xC0 ? 2 : (c & 0xF0) == 0xE0 ? 3 : (c & 0xF8) == 0xF0 ? 4 : 1;
        return (back + 1 >= need) ? end : end - 1 - back;
    }
    return end;
}

int lv_llama_chat(LVLlamaModel *value, const char *head, const char *system, const char *middle, const char *user,
                  const char *tail, int32_t max_tokens, int32_t n_ctx, LVTokenSink sink, void *sink_context,
                  LVCancellation *cancellation) {
    if (!value || !head || !system || !middle || !user || !tail || !sink || max_tokens < 1 || n_ctx < 256) return 1;
    if (value->vocab_only) return 2;
    LVDeadline deadline{cancellation, std::chrono::steady_clock::now() + std::chrono::seconds(120)};
    if (lv_should_abort(&deadline)) return 7;
    auto vocab = llama_model_get_vocab(value->model);
    std::vector<llama_token> tokens;
    if (!append_tokens(vocab, head, true, tokens) || !append_tokens(vocab, system, false, tokens)
        || !append_tokens(vocab, middle, true, tokens) || !append_tokens(vocab, user, false, tokens)
        || !append_tokens(vocab, tail, true, tokens)) return 12;
    // Nie über das Training hinaus: sonst Qualitätsverlust oder Absturz bei langen Kontexten.
    const int32_t n_train = llama_model_n_ctx_train(value->model);
    if (n_train > 0 && n_ctx > n_train) n_ctx = n_train;
    if (tokens.empty() || tokens.size() + static_cast<size_t>(max_tokens) > static_cast<size_t>(n_ctx)) return 4;

    auto cp = llama_context_default_params();
    cp.n_ctx = static_cast<uint32_t>(n_ctx);
    cp.n_batch = 512;
#if TARGET_OS_SIMULATOR
    cp.n_ubatch = 128; cp.offload_kqv = false;
#else
    cp.n_ubatch = 512; cp.offload_kqv = true;
#endif
    cp.n_threads = 4; cp.n_threads_batch = 4;
    cp.abort_callback = lv_should_abort; cp.abort_callback_data = &deadline;
    std::unique_ptr<llama_context, decltype(&llama_free)> context(llama_init_from_model(value->model, cp), llama_free);
    if (!context) return 3;

    for (size_t i = 0; i < tokens.size(); i += cp.n_batch) {
        if (lv_should_abort(&deadline)) return 7;
        const int32_t n = static_cast<int32_t>(std::min<size_t>(cp.n_batch, tokens.size() - i));
        if (llama_decode(context.get(), llama_batch_get_one(tokens.data() + i, n)) != 0) return lv_should_abort(&deadline) ? 7 : 5;
    }
    std::unique_ptr<llama_sampler, decltype(&llama_sampler_free)> sampler(llama_sampler_chain_init(llama_sampler_chain_default_params()), llama_sampler_free);
    if (!sampler) return 3;
    llama_sampler_chain_add(sampler.get(), llama_sampler_init_greedy());

    std::string pending;
    for (int32_t i = 0; i < max_tokens; ++i) {
        if (lv_should_abort(&deadline)) return 7;
        auto token = llama_sampler_sample(sampler.get(), context.get(), -1);
        if (llama_vocab_is_eog(vocab, token)) break;
        char piece[512];
        // Negativ = Puffer zu klein (llama.cpp meldet die nötige Länge als negative Zahl).
        const int size = llama_token_to_piece(vocab, token, piece, sizeof(piece), 0, true);
        if (size < 0) return 8;
        pending.append(piece, static_cast<size_t>(size));
        const size_t ready = utf8_complete_prefix(pending);
        if (ready > 0) {
            if (sink(pending.data(), static_cast<int32_t>(ready), sink_context) != 0) return 0;
            pending.erase(0, ready);
        }
        if (llama_decode(context.get(), llama_batch_get_one(&token, 1)) != 0) return lv_should_abort(&deadline) ? 7 : 5;
    }
    return 0;
}
