use tokenizers::Tokenizer;

use std::io::Read;
use std::sync::OnceLock;

pub(crate) static COMPRESSED_TOKENIZER: &[u8] = include_bytes!("../assets/tokenizer.json.zst");

pub(crate) static TOKENIZER: OnceLock<Tokenizer> = OnceLock::new();

pub fn estimate_accurate_tokens(text: &str) -> usize {
    let tok = TOKENIZER.get_or_init(|| get_tokenizer());

    tok.encode(text, true).map(|e| e.len()).unwrap_or(0)
}

pub(crate) fn get_tokenizer() -> Tokenizer {
    // Decompress zstd blob into a vector
    let mut decoder = zstd::Decoder::new(COMPRESSED_TOKENIZER).unwrap();
    let mut json_bytes = Vec::new();
    decoder.read_to_end(&mut json_bytes).unwrap();

    // Load tokenizer from the JSON buffer in memory
    Tokenizer::from_bytes(json_bytes).expect("Failed to load tokenizer")
}
