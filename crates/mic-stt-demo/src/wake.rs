//! Recognizes the wake word in a transcript.
//!
//! The speech model only knows English, so it spells “doufu” in several ways.

/// The word that wakes the demo.
pub const WAKE_WORD: &str = "doufu";

/// An English spelling that also counts as the wake word.
const ALIAS: &[u8] = b"tofu";

/// Longest word that is considered, in letters.
const MAX_LETTERS: usize = 16;

/// Spelling replacements that bring similar sounds together, applied in order.
const SPELLINGS: [(&[u8], &[u8]); 4] = [
    (b"ough", b"o"),
    (b"oo", b"u"),
    (b"ow", b"ou"),
    (b"ph", b"f"),
];

/// Tells whether `transcript` contains the wake word.
///
/// A word, or two adjacent words together, must be `tofu`, within one edit of
/// [`WAKE_WORD`], or sound like it, after the spelling replacements.
pub fn is_wake_word(transcript: &str) -> bool {
    let words = transcript
        .split(|c: char| !c.is_ascii_alphabetic())
        .filter(|word| !word.is_empty());

    let mut previous: Option<Word> = None;
    for word in words {
        // A word too long to hold is skipped, and separates its neighbors.
        if word.len() > MAX_LETTERS {
            previous = None;
            continue;
        }
        let word = Word::normalize(word);
        if matches(word.letters()) {
            return true;
        }
        if let Some(previous) = previous {
            let mut joined = [0u8; 2 * MAX_LETTERS];
            let len = previous.len + word.len;
            joined[..previous.len].copy_from_slice(previous.letters());
            joined[previous.len..len].copy_from_slice(word.letters());
            if matches(&joined[..len]) {
                return true;
            }
        }
        previous = Some(word);
    }
    false
}

fn matches(candidate: &[u8]) -> bool {
    candidate == ALIAS
        || levenshtein(candidate, WAKE_WORD.as_bytes()) <= 1
        || sounds_like_wake_word(candidate)
}

/// Tells whether `word` is `d`, one or more of `a`, `o` and `u`, an optional
/// `l` or `r`, `f`, and then only `a`, `o` or `u`.
///
/// The bundled model spells the spoken wake word this way, as in “dof”,
/// “darf”, “dalfu” or “do fo”. The front vowels and the endings after `f` are
/// excluded so that words such as “deaf”, “defy” or “dolphin” do not wake the
/// demo.
fn sounds_like_wake_word(word: &[u8]) -> bool {
    let is_vowel = |letter: &u8| matches!(letter, b'a' | b'o' | b'u');
    let Some(rest) = word.strip_prefix(b"d") else {
        return false;
    };
    let vowels = rest.iter().take_while(|letter| is_vowel(letter)).count();
    if vowels == 0 {
        return false;
    }
    let rest = &rest[vowels..];
    let rest = rest
        .strip_prefix(b"l")
        .or_else(|| rest.strip_prefix(b"r"))
        .unwrap_or(rest);
    rest.strip_prefix(b"f")
        .is_some_and(|tail| tail.iter().all(is_vowel))
}

/// A lowercase word of at most [`MAX_LETTERS`] letters.
#[derive(Clone, Copy)]
struct Word {
    buffer: [u8; MAX_LETTERS],
    len: usize,
}

impl Word {
    /// Lowercases `word`, which holds ASCII letters only, and respells it.
    fn normalize(word: &str) -> Self {
        let mut normalized = Self {
            buffer: [0; MAX_LETTERS],
            len: word.len(),
        };
        for (to, from) in normalized.buffer.iter_mut().zip(word.bytes()) {
            *to = from.to_ascii_lowercase();
        }
        for (from, to) in SPELLINGS {
            normalized.replace(from, to);
        }
        normalized
    }

    fn letters(&self) -> &[u8] {
        &self.buffer[..self.len]
    }

    /// Replaces `from` with `to`, which is not longer, from left to right.
    fn replace(&mut self, from: &[u8], to: &[u8]) {
        let (mut read, mut write) = (0, 0);
        while read < self.len {
            if self.buffer[read..self.len].starts_with(from) {
                self.buffer[write..write + to.len()].copy_from_slice(to);
                read += from.len();
                write += to.len();
            } else {
                self.buffer[write] = self.buffer[read];
                read += 1;
                write += 1;
            }
        }
        self.len = write;
    }
}

/// Edit distance of two words of at most `2 * MAX_LETTERS` letters.
fn levenshtein(a: &[u8], b: &[u8]) -> u8 {
    // No distance exceeds the 32 letters of the longer word, so bytes hold it
    // and the two rows stay small on the stack.
    let mut previous = [0u8; 2 * MAX_LETTERS + 1];
    for (distance, j) in previous.iter_mut().zip(0..) {
        *distance = j;
    }
    let mut current = previous;
    for (&x, row) in a.iter().zip(1..) {
        current[0] = row;
        for (j, &y) in b.iter().enumerate() {
            let substitution = previous[j] + u8::from(x != y);
            current[j + 1] = substitution.min(previous[j + 1] + 1).min(current[j] + 1);
        }
        previous = current;
    }
    previous[b.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_match_doufu_and_close_transcripts() {
        for t in [
            "doufu",
            "Doufu.",
            "dou fu",
            "do foo",
            "dough foo",
            "dofu",
            "dowfu",
            "tofu",
            "Tofu!",
            "hey doufu",
            "doufu turn on the lamp",
            // Transcripts of “doufu” that the bundled model produced for 100
            // clips of ten synthetic voices, through `Segmenter` and
            // `Transcriber` with the language model and the firmware's beam
            // search.
            "dalfou",
            "dalfu",
            "darf",
            "do f",
            "do fo",
            "dof",
            "dolf",
            "dolph",
            "dophu",
            "dorf",
            "du f",
            "du fo",
            "dufu",
        ] {
            assert!(is_wake_word(t), "{t:?} should match");
        }
    }

    #[test]
    fn should_reject_other_transcripts() {
        for t in [
            "",
            "   ",
            "hello",
            "turn on the lamp",
            "do",
            "food",
            "toffee",
            "do you",
            "you",
            "deaf",
            "defy",
            "doff",
            "duff",
            "doubt",
            "doubtful",
            "down",
            "dour",
            "dolphin",
            "do for",
            "do few",
            "豆腐",
            "doufudoufudoufudoufu",
            "dou supercalifragilisticexpialidocious fu",
        ] {
            assert!(!is_wake_word(t), "{t:?} should not match");
        }
    }
}
