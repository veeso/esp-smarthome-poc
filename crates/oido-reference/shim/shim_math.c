// Exports the inline functions of `tasr_math.h` to the Rust parity tests.
#include "tasr_math.h"

float ref_expf(float x) { return tasr_expf(x); }
float ref_logf(float x) { return tasr_logf(x); }
