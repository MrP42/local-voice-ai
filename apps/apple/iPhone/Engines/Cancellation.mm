#include "Cancellation.hpp"
LVCancellation *lv_cancel_create(void) { return new LVCancellation(); }
void lv_cancel_request(LVCancellation *value) { if (value) value->cancelled.store(true); }
void lv_cancel_destroy(LVCancellation *value) { delete value; }
