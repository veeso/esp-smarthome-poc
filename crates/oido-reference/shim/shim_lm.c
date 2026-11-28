// Includes the upstream language model and beam search so that their static
// functions are reachable from the Rust parity tests.
#include "../vendor/tinyasr_lm.c"

// Advances `n <= 16` language model states by one token each, from `h_prev`.
void ref_lm_steps(const tasr_lm_t *lm, const int *tokens, const float *h_prev, int n, float *h, float *logp)
{
    tasr_decoder_t *d = tasr_decoder_create(lm, lm->V, 1, 1, 0.f, 0.f);
    int parent[LMB], token[LMB], out[LMB];
    for (int i = 0; i < n; i++) {
        memcpy(d->st_h + (size_t)i * lm->H, h_prev + (size_t)i * lm->H, sizeof(float) * lm->H);
        parent[i] = i;
        token[i] = tokens[i];
        out[i] = LMB + i;
    }
    lm_steps(d, parent, token, out, n);
    memcpy(h, d->st_h + (size_t)LMB * lm->H, sizeof(float) * (size_t)n * lm->H);
    memcpy(logp, d->st_lp + (size_t)LMB * lm->V, sizeof(float) * (size_t)n * lm->V);
    tasr_decoder_free(d);
}
int ref_lm_hidden_size(const tasr_lm_t *lm) { return lm->H; }
