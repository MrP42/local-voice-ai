#include "Cancellation.hpp"
static std::atomic<bool> inference_busy{false};
int32_t lv_inference_try_acquire(void) {
    bool expected = false;
    return inference_busy.compare_exchange_strong(expected, true) ? 1 : 0;
}
void lv_inference_release(void) { inference_busy.store(false); }
LVCancellation *lv_cancel_create(void) { return new LVCancellation(); }
void lv_cancel_request(LVCancellation *value) { if (value) value->cancelled.store(true); }
void lv_cancel_destroy(LVCancellation *value) { delete value; }
