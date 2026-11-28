//! The CTC head, its log-softmax, greedy decoding, and detokenization.

// Ported from esp32/components/tinyasr/tasr_nemo.c:290-292 and 1192-1241 at
// commit c8d60325ba8bffd4bad65a9ac63c713ee64d1aae.

use alloc::string::String;

use crate::conformer::Scratch;
use crate::frontend::Frames;
use crate::mathx::{Tables, logf};
use crate::model::Model;

/// Rows that one pass of the head takes.
const ROW_BLOCK: usize = 64;

/// Logit margin of the blank above which `p(blank) > 0.999` with 1024 tokens.
const BLANK_MARGIN: f32 = 14.0;

/// Calls `on_row` with the raw logits of each frame; the blank is last.
pub(crate) fn head_blocks(
    model: &Model<'_>,
    x: &Frames,
    scratch: &mut Scratch,
    on_row: &mut dyn FnMut(&[f32]),
) {
    for rows in x.data.chunks(ROW_BLOCK * x.d) {
        let logits = scratch.logits(&model.head, rows, rows.len() / x.d);
        logits.chunks_exact(model.head.n).for_each(&mut *on_row);
    }
}

/// The most likely class of a frame, the lowest of equals.
pub(crate) fn greedy(row: &[f32]) -> u32 {
    let mut best = 0;
    for (class, &value) in row.iter().enumerate() {
        if value > row[best] {
            best = class;
        }
    }
    best as u32
}

/// Log-probabilities of a frame with the blank moved to index 0.
///
/// A blank-dominated frame only gets `out[0] = 0`, which is all that the beam
/// search reads of it.
pub(crate) fn ctc_log_probs(row: &[f32], tables: &Tables, out: &mut [f32]) {
    let blank = row.len() - 1;
    let (mut best, mut max, mut second) = (0, row[0], -3.0e38f32);
    for (class, &value) in row.iter().enumerate().skip(1) {
        if value > max {
            second = max;
            max = value;
            best = class;
        } else if value > second {
            second = value;
        }
    }
    if best == blank && max - second >= BLANK_MARGIN {
        out[0] = 0.0;
        return;
    }
    let mut sum = 0.0f32;
    for &value in row {
        sum += tables.fast_expneg(max - value);
    }
    let log_sum = max + logf(sum);
    out[0] = row[blank] - log_sum;
    for (out, &value) in out[1..].iter_mut().zip(&row[..blank]) {
        *out = value - log_sum;
    }
}

/// Replaces `out` with the text of `tokens`, without its leading space.
pub(crate) fn detokenize(model: &Model<'_>, tokens: &[u32], out: &mut String) {
    out.clear();
    out.extend(tokens.iter().filter_map(|&token| model.token(token)));
    if out.starts_with(' ') {
        out.remove(0);
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec::Vec;
    use alloc::{format, vec};

    use super::*;
    use crate::mathx::tests::XorShift32;

    #[test]
    fn ctc_log_probs_should_match_the_reference() {
        let tables = Tables::new();
        let mut rng = XorShift32::new();
        let mut rows: Vec<Vec<f32>> = (0..1_000)
            .map(|i| {
                let spread = rng.uniform(0.5, 30.0);
                let mut row: Vec<f32> = (0..1025).map(|_| rng.uniform(-spread, spread)).collect();
                // Every other row has a dominant blank.
                if i % 2 == 0 {
                    row[1024] = spread + rng.uniform(0.0, 30.0);
                }
                row
            })
            .collect();
        let mut margin = vec![0.0; 1025];
        margin[1024] = 14.0;
        rows.push(margin);

        let (mut shortcuts, mut full) = (0, 0);
        for row in &rows {
            // The sentinel shows which values a branch leaves alone.
            let (mut actual, mut expected) = (vec![123.0; 1025], vec![123.0; 1025]);
            ctc_log_probs(row, &tables, &mut actual);
            oido_reference::ctc_log_probs(row, &mut expected);
            assert!(
                actual
                    .iter()
                    .zip(&expected)
                    .all(|(a, b)| a.to_bits() == b.to_bits())
            );
            if actual[1].to_bits() == 123.0f32.to_bits() {
                shortcuts += 1;
            } else {
                full += 1;
            }
        }
        assert!(shortcuts > 0 && full > 0, "{shortcuts} and {full}");
    }

    #[test]
    fn detokenize_should_strip_one_leading_space_and_skip_empty_tokens() {
        let model = Model::parse(oido_reference::model_blob()).unwrap();
        let find = |wanted: &dyn Fn(&str) -> bool| {
            (0..1024)
                .find(|&id| wanted(model.token(id).unwrap()))
                .unwrap()
        };
        let word = find(&|token| token.starts_with(' ') && token.len() > 1);
        let control = find(&str::is_empty);

        let mut out = String::from("stale");
        detokenize(&model, &[word, control, word], &mut out);

        let text = model.token(word).unwrap();
        assert_eq!(out, format!("{first}{text}", first = &text[1..]));
    }
}
