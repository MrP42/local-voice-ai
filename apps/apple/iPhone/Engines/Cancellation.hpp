#pragma once
#include "LVEngines.h"
#include <atomic>
#include <chrono>
struct LVCancellation { std::atomic<bool> cancelled{false}; };
struct LVDeadline {
    LVCancellation *token;
    std::chrono::steady_clock::time_point deadline;
};
inline bool lv_should_abort(void *value) {
    const auto *state = static_cast<LVDeadline *>(value);
    return (state->token && state->token->cancelled.load()) || std::chrono::steady_clock::now() > state->deadline;
}
