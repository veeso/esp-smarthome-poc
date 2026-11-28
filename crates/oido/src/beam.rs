//! CTC prefix beam search with language model fusion.

// Ported from esp32/components/tinyasr/tinyasr_lm.c:86-423 at commit
// c8d60325ba8bffd4bad65a9ac63c713ee64d1aae.

use alloc::boxed::Box;
use alloc::vec::Vec;

use crate::lm::{LanguageModel, LmScratch};
use crate::mathx::{expf, logf};
use crate::transcriber::{TranscribeError, zeroed};

/// Largest beam width.
pub const MAX_BEAM_WIDTH: usize = 16;

/// Largest number of tokens that one frame adds to a beam.
pub const MAX_TOP_K: usize = 16;

/// Beam search weight when neither the caller nor the file has one.
const FALLBACK_WEIGHT: f32 = 0.6;

/// Token bonus when neither the caller nor the file has one.
const FALLBACK_BONUS: f32 = 2.5;

/// Log-probability of an impossible path.
const IMPOSSIBLE: f32 = -1e30;

/// `ln(0.999)`: above it a frame only extends the beams with a blank.
const BLANK_SKIP: f32 = -0.001_000_500_3;

/// Prefixes that one utterance can create.
const MAX_NODES: usize = 16_384;

/// Slots of the table that finds a child of a prefix; a power of two.
const HASH_SLOTS: usize = 32_768;

/// Language model states that can be alive at once.
const STATE_SLOTS: usize = 2 * MAX_BEAM_WIDTH + 2;

/// Candidates of one frame: each beam with a blank and every token twice.
const MAX_CANDIDATES: usize = MAX_BEAM_WIDTH * (1 + 2 * MAX_TOP_K);

/// Marks a missing prefix, slot, or token.
const NONE: i32 = -1;

/// Settings of the beam search.
///
/// # Examples
///
/// ```
/// use oido::BeamConfig;
///
/// let config = BeamConfig {
///     beam_width: 4,
///     ..BeamConfig::default()
/// };
/// assert_eq!(config.top_k, 3);
/// ```
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BeamConfig {
    /// Prefixes kept after each frame, from 1 to [`MAX_BEAM_WIDTH`].
    pub beam_width: usize,
    /// Tokens tried on each frame, from 1 to [`MAX_TOP_K`].
    pub top_k: usize,
    /// Weight of the language model; `None` takes its recommendation.
    pub lm_weight: Option<f32>,
    /// Score added for each token; `None` takes the recommendation.
    pub token_bonus: Option<f32>,
}

impl Default for BeamConfig {
    fn default() -> Self {
        Self {
            beam_width: 2,
            top_k: 3,
            lm_weight: None,
            token_bonus: None,
        }
    }
}

/// A prefix in the trie.
#[derive(Clone, Copy, Debug, Default)]
struct Node {
    parent: i32,
    token: i32,
    /// Slot of the language model state, or [`NONE`].
    state: i32,
    /// Language model log-probability of the whole prefix.
    lm: f32,
    len: u32,
}

#[derive(Clone, Copy, Debug)]
struct Beam {
    node: usize,
    /// Log-probability of the paths that end in a blank.
    blank: f32,
    /// Log-probability of the paths that end in a token.
    token: f32,
}

#[derive(Clone, Copy, Debug, Default)]
struct Candidate {
    /// The existing prefix, or `None` for a child not yet in the trie.
    node: Option<usize>,
    parent: i32,
    token: i32,
    blank: f32,
    non_blank: f32,
    lm: f32,
    len: u32,
    taken: bool,
}

/// Language model states of the live prefixes.
#[derive(Debug)]
struct States<'m> {
    lm: &'m LanguageModel<'m>,
    scratch: Box<LmScratch>,
    /// `[STATE_SLOTS][hidden]`.
    hidden: Vec<f32>,
    /// `[STATE_SLOTS][vocab]`.
    logp: Vec<f32>,
    free: Vec<usize>,
    /// Inputs and outputs of one batch of steps.
    batch_previous: Vec<f32>,
    batch_hidden: Vec<f32>,
    batch_logp: Vec<f32>,
}

impl States<'_> {
    fn reset(&mut self) {
        self.free.clear();
        self.free.extend((0..STATE_SLOTS).rev());
    }

    /// Writes the state after `token` from `parent` into slot `out`, for each step.
    fn step(&mut self, parents: &[i32], tokens: &[u32], outputs: &[usize]) {
        let width = self.lm.hidden_size();
        let vocab = self.lm.vocab_size();
        for (row, &parent) in self.batch_previous.chunks_exact_mut(width).zip(parents) {
            match usize::try_from(parent) {
                Ok(slot) => row.copy_from_slice(&self.hidden[slot * width..][..width]),
                Err(_) => row.fill(0.0),
            }
        }
        let count = tokens.len();
        self.lm.steps(
            tokens,
            &self.batch_previous[..count * width],
            &mut self.batch_hidden[..count * width],
            &mut self.batch_logp,
            &mut self.scratch,
        );
        for (nth, &slot) in outputs.iter().enumerate() {
            self.hidden[slot * width..][..width]
                .copy_from_slice(&self.batch_hidden[nth * width..][..width]);
            self.logp[slot * vocab..][..vocab]
                .copy_from_slice(&self.batch_logp[nth * vocab..][..vocab]);
        }
    }
}

/// CTC prefix beam search, optionally fused with a language model.
#[derive(Debug)]
pub(crate) struct BeamDecoder<'m> {
    states: Option<States<'m>>,
    classes: usize,
    width: usize,
    top_k: usize,
    lm_weight: f32,
    token_bonus: f32,
    nodes: Vec<Node>,
    node_count: usize,
    /// Node of each `(parent, token)` pair, by open addressing.
    children: Vec<i32>,
    beams: [Beam; MAX_BEAM_WIDTH],
    beam_count: usize,
    candidates: Vec<Candidate>,
}

impl<'m> BeamDecoder<'m> {
    /// A decoder over `classes` CTC classes, the blank at index 0.
    pub(crate) fn new(
        lm: Option<&'m LanguageModel<'m>>,
        classes: usize,
        config: BeamConfig,
    ) -> Result<Self, TranscribeError> {
        if !(1..=MAX_BEAM_WIDTH).contains(&config.beam_width)
            || !(1..=MAX_TOP_K).contains(&config.top_k)
        {
            return Err(TranscribeError::InvalidBeamConfig);
        }
        if let Some(lm) = lm
            && lm.vocab_size() != classes
        {
            return Err(TranscribeError::VocabularyMismatch {
                model: classes,
                language_model: lm.vocab_size(),
            });
        }
        let recommended =
            |value: Option<f32>, fallback| value.filter(|&v| v > 0.0).unwrap_or(fallback);
        let lm_weight = config.lm_weight.unwrap_or_else(|| {
            recommended(lm.map(LanguageModel::recommended_weight), FALLBACK_WEIGHT)
        });
        let token_bonus = config.token_bonus.unwrap_or_else(|| {
            recommended(
                lm.map(LanguageModel::recommended_token_bonus),
                FALLBACK_BONUS,
            )
        });
        let states = match lm {
            Some(lm) => Some(States {
                lm,
                scratch: Box::new(LmScratch::new(lm, MAX_BEAM_WIDTH)?),
                hidden: zeroed(STATE_SLOTS * lm.hidden_size())?,
                logp: zeroed(STATE_SLOTS * lm.vocab_size())?,
                free: Vec::with_capacity(STATE_SLOTS),
                batch_previous: zeroed(MAX_BEAM_WIDTH * lm.hidden_size())?,
                batch_hidden: zeroed(MAX_BEAM_WIDTH * lm.hidden_size())?,
                batch_logp: zeroed(MAX_BEAM_WIDTH * lm.vocab_size())?,
            }),
            None => None,
        };
        let mut candidates = Vec::new();
        candidates
            .try_reserve_exact(MAX_CANDIDATES)
            .map_err(|_error| TranscribeError::OutOfMemory)?;
        let mut decoder = Self {
            states,
            classes,
            width: config.beam_width,
            top_k: config.top_k,
            lm_weight,
            token_bonus,
            nodes: zeroed(MAX_NODES)?,
            node_count: 0,
            children: zeroed(HASH_SLOTS)?,
            beams: [Beam {
                node: 0,
                blank: 0.0,
                token: IMPOSSIBLE,
            }; MAX_BEAM_WIDTH],
            beam_count: 0,
            candidates,
        };
        decoder.reset();
        Ok(decoder)
    }

    /// Starts a new utterance from the empty prefix.
    pub(crate) fn reset(&mut self) {
        self.node_count = 1;
        self.nodes[0] = Node {
            parent: NONE,
            token: NONE,
            state: NONE,
            lm: 0.0,
            len: 0,
        };
        self.children.fill(NONE);
        if let Some(states) = &mut self.states {
            states.reset();
            if let Some(slot) = states.free.pop() {
                // The last token of the language model begins a sentence.
                let begin = states.lm.vocab_size() as u32 - 1;
                states.step(&[NONE], &[begin], &[slot]);
                self.nodes[0].state = slot as i32;
            }
        }
        self.beams[0] = Beam {
            node: 0,
            blank: 0.0,
            token: IMPOSSIBLE,
        };
        self.beam_count = 1;
    }

    /// Advances by one frame of log-probabilities with the blank at index 0.
    ///
    /// When `logp[0]` exceeds `ln(0.999)` no other value is read.
    pub(crate) fn step(&mut self, logp: &[f32]) {
        if logp[0] > BLANK_SKIP {
            for beam in &mut self.beams[..self.beam_count] {
                beam.blank = log_add(beam.blank, beam.token) + logp[0];
                beam.token = IMPOSSIBLE;
            }
            return;
        }
        self.expand(logp);
        let old = self.beams;
        let old_count = self.beam_count;
        self.select();
        self.update_states(&old[..old_count]);
    }

    /// Collects every way to extend the beams by this frame.
    fn expand(&mut self, logp: &[f32]) {
        // The `top_k` most likely tokens, by insertion sort.
        let mut best = [(0usize, 0.0f32); MAX_TOP_K];
        let mut count = 0;
        for (class, &value) in logp.iter().enumerate().take(self.classes).skip(1) {
            if count < self.top_k || value > best[count - 1].1 {
                let mut at = if count < self.top_k {
                    count += 1;
                    count - 1
                } else {
                    count - 1
                };
                while at > 0 && best[at - 1].1 < value {
                    best[at] = best[at - 1];
                    at -= 1;
                }
                best[at] = (class, value);
            }
        }

        self.candidates.clear();
        for nth in 0..self.beam_count {
            let beam = self.beams[nth];
            let node = self.nodes[beam.node];
            let total = log_add(beam.blank, beam.token);
            // The blank keeps the prefix.
            let kept = self.existing(beam.node);
            self.merge(kept, total + logp[0], IMPOSSIBLE);
            for &(class, value) in &best[..count] {
                let token = class as i32 - 1;
                let repeat = token == node.token;
                if repeat {
                    // A repeat collapses into the same prefix.
                    self.merge(kept, IMPOSSIBLE, beam.token + value);
                }
                // A new token, or a repeat after a blank.
                let from = if repeat { beam.blank } else { total };
                let child = match self.find_child(beam.node, token) {
                    Some(child) => self.existing(child),
                    None => Candidate {
                        node: None,
                        parent: beam.node as i32,
                        token,
                        lm: node.lm + self.lm_logp(node.state, class - 1),
                        len: node.len + 1,
                        ..Candidate::default()
                    },
                };
                self.merge(child, IMPOSSIBLE, from + value);
            }
        }
    }

    fn lm_logp(&self, state: i32, token: usize) -> f32 {
        match (&self.states, usize::try_from(state)) {
            (Some(states), Ok(slot)) => states.logp[slot * states.lm.vocab_size() + token],
            _ => 0.0,
        }
    }

    fn existing(&self, index: usize) -> Candidate {
        let node = self.nodes[index];
        Candidate {
            node: Some(index),
            parent: node.parent,
            token: node.token,
            lm: node.lm,
            len: node.len,
            ..Candidate::default()
        }
    }

    /// Adds the path probabilities to the candidate of the same prefix.
    fn merge(&mut self, candidate: Candidate, blank: f32, non_blank: f32) {
        let same = |other: &Candidate| match candidate.node {
            Some(node) => other.node == Some(node),
            None => {
                other.node.is_none()
                    && other.parent == candidate.parent
                    && other.token == candidate.token
            }
        };
        let found = self.candidates.iter().position(same);
        let at = found.unwrap_or_else(|| {
            self.candidates.push(Candidate {
                blank: IMPOSSIBLE,
                non_blank: IMPOSSIBLE,
                ..candidate
            });
            self.candidates.len() - 1
        });
        let merged = &mut self.candidates[at];
        merged.blank = log_add(merged.blank, blank);
        merged.non_blank = log_add(merged.non_blank, non_blank);
    }

    /// Keeps the best candidates as the new beams.
    fn select(&mut self) {
        self.beam_count = 0;
        for _ in 0..self.width.min(self.candidates.len()) {
            let mut best = None;
            let mut best_score = IMPOSSIBLE;
            for (at, candidate) in self.candidates.iter().enumerate() {
                if candidate.taken {
                    continue;
                }
                let score = self.score(
                    log_add(candidate.blank, candidate.non_blank),
                    candidate.lm,
                    candidate.len,
                );
                if score > best_score {
                    best_score = score;
                    best = Some(at);
                }
            }
            let Some(at) = best else {
                break;
            };
            self.candidates[at].taken = true;
            let candidate = self.candidates[at];
            let node = candidate.node.or_else(|| self.make_child(&candidate));
            if let Some(node) = node {
                self.beams[self.beam_count] = Beam {
                    node,
                    blank: candidate.blank,
                    token: candidate.non_blank,
                };
                self.beam_count += 1;
            }
        }
    }

    fn score(&self, paths: f32, lm: f32, len: u32) -> f32 {
        paths + self.lm_weight * lm + self.token_bonus * len as f32
    }

    /// Computes the states of new beams and frees those of dropped ones.
    fn update_states(&mut self, old: &[Beam]) {
        let Some(states) = &mut self.states else {
            return;
        };
        let mut parents = [NONE; MAX_BEAM_WIDTH];
        let mut tokens = [0u32; MAX_BEAM_WIDTH];
        let mut outputs = [0usize; MAX_BEAM_WIDTH];
        let mut owners = [0usize; MAX_BEAM_WIDTH];
        let mut count = 0;
        let beams = &self.beams[..self.beam_count];
        for beam in beams {
            let node = self.nodes[beam.node];
            if node.state != NONE || node.parent < 0 {
                continue;
            }
            let Some(slot) = states.free.pop() else {
                continue;
            };
            parents[count] = self.nodes[node.parent as usize].state;
            tokens[count] = node.token as u32;
            outputs[count] = slot;
            owners[count] = beam.node;
            count += 1;
        }
        if count > 0 {
            states.step(&parents[..count], &tokens[..count], &outputs[..count]);
        }
        for (&owner, &slot) in owners.iter().zip(&outputs).take(count) {
            self.nodes[owner].state = slot as i32;
        }
        for dropped in old {
            if beams.iter().any(|beam| beam.node == dropped.node) {
                continue;
            }
            let node = &mut self.nodes[dropped.node];
            if let Ok(slot) = usize::try_from(node.state) {
                if states.free.len() < STATE_SLOTS {
                    states.free.push(slot);
                }
                node.state = NONE;
            }
        }
    }

    /// Table slots to probe for the child `token` of `parent`, in order.
    fn probes(parent: usize, token: i32) -> impl Iterator<Item = usize> {
        let key = (parent as u32).wrapping_mul(2_654_435_761)
            ^ token.cast_unsigned().wrapping_mul(40_503);
        (0..HASH_SLOTS).map(move |probe| (key as usize).wrapping_add(probe) & (HASH_SLOTS - 1))
    }

    fn find_child(&self, parent: usize, token: i32) -> Option<usize> {
        for at in Self::probes(parent, token) {
            let index = usize::try_from(self.children[at]).ok()?;
            let node = self.nodes[index];
            if node.parent == parent as i32 && node.token == token {
                return Some(index);
            }
        }
        None
    }

    /// Adds a prefix to the trie, or returns `None` when the trie is full.
    fn make_child(&mut self, candidate: &Candidate) -> Option<usize> {
        if self.node_count >= MAX_NODES {
            return None;
        }
        let parent = usize::try_from(candidate.parent).ok()?;
        let at = Self::probes(parent, candidate.token).find(|&at| self.children[at] == NONE)?;
        let index = self.node_count;
        self.node_count += 1;
        self.nodes[index] = Node {
            parent: candidate.parent,
            token: candidate.token,
            state: NONE,
            lm: candidate.lm,
            len: candidate.len,
        };
        self.children[at] = index as i32;
        Some(index)
    }

    /// Replaces `out` with the tokens of the best prefix.
    pub(crate) fn best(&self, out: &mut Vec<u32>) {
        let mut best = 0;
        let mut best_score = IMPOSSIBLE;
        for (at, beam) in self.beams[..self.beam_count].iter().enumerate() {
            let node = self.nodes[beam.node];
            let score = self.score(log_add(beam.blank, beam.token), node.lm, node.len);
            if score > best_score {
                best_score = score;
                best = at;
            }
        }
        out.clear();
        let mut index = self.beams[best].node;
        while index > 0 {
            let node = self.nodes[index];
            out.push(node.token as u32);
            index = node.parent as usize;
        }
        out.reverse();
    }
}

/// `ln(e^a + e^b)`.
fn log_add(a: f32, b: f32) -> f32 {
    let (a, b) = if a < b { (b, a) } else { (a, b) };
    if b <= IMPOSSIBLE * 0.5 {
        return a;
    }
    a + logf(1.0 + expf(b - a))
}
