# oido

`no_std` Rust port of the [Oído](https://github.com/lokutor-ai/oido) on-device
English speech-to-text engine, at upstream commit
`c8d60325ba8bffd4bad65a9ac63c713ee64d1aae`.

The crate runs on the ESP32-S3 and on regular computers. It needs `alloc`. On the
same computer, transcripts and logits match the C engine bit for bit, and
segmenter samples match it within one unit.

## API

| Item            | Purpose                                               |
| --------------- | ----------------------------------------------------- |
| `Model`         | Parses a TNM1 acoustic model in place.                |
| `LanguageModel` | Parses a TLM1 language model in place.                |
| `Transcriber`   | Transcribes an utterance, greedily or by beam search. |
| `Segmenter`     | Cuts live audio into utterances.                      |
| `Aligned`       | Holds model bytes at the 16-byte alignment they need. |

```rust,ignore
use oido::{BeamConfig, Decoding, LanguageModel, Model, Transcriber};

let model = Model::parse(model_blob)?;
let lm = LanguageModel::parse(lm_blob)?;
let mut transcriber = Transcriber::new(
    &model,
    Decoding::Beam {
        language_model: Some(&lm),
        config: BeamConfig::default(),
    },
)?;
let text = transcriber.transcribe(pcm)?;
```

The audio is one utterance of 16 kHz mono PCM16. The working memory grows with
its length, so the chip needs PSRAM.

## License

The code is licensed under the GNU GPL version 3, see `LICENSE`. It is a port of
Oído, Copyright (C) 2026 Hashing Works, S.L.L. (Lokutor).

The model files are not part of the crate. They are licensed under CC BY-NC-SA
4.0 with Lokutor's terms of use, which exclude commercial use without a written
license.
