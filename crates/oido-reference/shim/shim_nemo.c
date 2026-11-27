// Includes the upstream utterance engine so that its static functions and its
// per-stage tensor hashes are reachable from the Rust parity tests.
#include <stdarg.h>
#include <stdio.h>
#include <string.h>
int ref_printf(const char *fmt, ...);
#define TASR_DEBUG_SUMS 1
#define printf ref_printf
#include "../vendor/tasr_nemo.c"
#undef printf

static char dbg_buf[1 << 16];
static size_t dbg_len;

int ref_printf(const char *fmt, ...)
{
    va_list args;
    va_start(args, fmt);
    const int n = vsnprintf(dbg_buf + dbg_len, sizeof(dbg_buf) - dbg_len, fmt, args);
    va_end(args);
    if (n > 0) {
        const size_t room = sizeof(dbg_buf) - dbg_len - 1;
        dbg_len += (size_t)n < room ? (size_t)n : room;
    }
    return n;
}

// Copies the captured lines into `out` as a NUL-terminated string and clears them.
void ref_dbg_take(char *out, size_t cap)
{
    const size_t n = dbg_len < cap - 1 ? dbg_len : cap - 1;
    memcpy(out, dbg_buf, n);
    out[n] = 0;
    dbg_len = 0;
}

void ref_init_tables(void)
{
    nsig_init();
    nexp_init();
    tasr_fast_init();
}
float ref_nsig(float x) { return nsig(x); }
float ref_nexp_tab(int i) { return nexp_tab[i]; }
float ref_qvec(const float *x, int n, int8_t *q, int np) { return qvec(x, n, q, np); }
// Copies token `i` into `out` and returns its length, or -1 when it does not fit.
int ref_token(const tasr_nemo_t *m, int i, char *out, int cap)
{
    if (i < 0 || i >= m->V || m->tok_len[i] > cap) return -1;
    memcpy(out, m->tok[i], m->tok_len[i]);
    return m->tok_len[i];
}
// Writes the normalized log-mel features, 80 per frame, and returns the frames.
int ref_features(const tasr_nemo_t *m, const int16_t *pcm, int n, float *out, int cap_frames)
{
    int frames = 0;
    float *features = nemo_features(m, pcm, n, &frames);
    if (frames <= cap_frames) memcpy(out, features, sizeof(float) * (size_t)frames * NMEL);
    tasr_free(features);
    return frames;
}
// Runs the blank shortcut and log-softmax of one logit row through `s_emit`,
// writing the values that it computes into `out`.
void ref_ctc_log_probs(const float *row, int v1, float *out)
{
    static struct tasr_nemo model;
    static struct tasr_nemo_stream stream;
    memset(&model, 0, sizeof(model));
    memset(&stream, 0, sizeof(stream));
    model.V = v1 - 1;
    stream.m = &model;
    stream.lp = out;
    stream.dec = tasr_decoder_create(NULL, v1, 1, 1, 0.f, 0.f);
    s_emit(&stream, row);
    tasr_decoder_free(stream.dec);
}
