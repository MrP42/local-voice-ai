#pragma once
#include "LVEngines.h"
#include <atomic>
#include <chrono>
#include <thread>
struct LVCancellation { std::atomic<bool> cancelled{false}; };
struct LVDeadline {
    LVCancellation *token;
    std::chrono::steady_clock::time_point deadline;
};
inline bool lv_should_abort(void *value) {
    const auto *state = static_cast<LVDeadline *>(value);
    return (state->token && state->token->cancelled.load()) || std::chrono::steady_clock::now() > state->deadline;
}

// Die Freigabe ist nicht an einen Thread gebunden (Foundation Models suspendiert).
struct LVInferenceLease {
    bool acquired = false;
    explicit LVInferenceLease(LVCancellation *token = nullptr) {
        while (!(token && token->cancelled.load())) {
            if (lv_inference_try_acquire()) { acquired = true; break; }
            std::this_thread::sleep_for(std::chrono::milliseconds(20));
        }
    }
    ~LVInferenceLease() { if (acquired) lv_inference_release(); }
    LVInferenceLease(const LVInferenceLease &) = delete;
    LVInferenceLease &operator=(const LVInferenceLease &) = delete;
};
