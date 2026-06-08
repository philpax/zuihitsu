//! The embedder seam. The real embedder (jina v5 over the OpenAI-compatible endpoint, 1024-dim) is
//! wired in Stage 5; tests use a deterministic fake so identical text embeds identically and the
//! vector index can be exercised without a model (spec §Testability).

use async_trait::async_trait;

use super::ModelError;

/// A dense embedding vector. Dimensionality is fixed per embedder.
pub type Embedding = Vec<f32>;

/// Turns text into vectors. Errors share [`ModelError`] with the model seam, since both are
/// inference over the same serving layer.
#[async_trait]
pub trait Embedder: Send + Sync {
    fn dimensions(&self) -> usize;
    /// The id of the model producing these embeddings, stamped onto each vector as provenance so a
    /// mixed-embedding-space state is detectable (spec §Storage → vector store).
    fn model_id(&self) -> &str;
    async fn embed(&self, inputs: &[String]) -> Result<Vec<Embedding>, ModelError>;
}

/// A deterministic fake: each input hashes to a fixed unit vector, so identical strings embed
/// identically and distinct strings embed distinctly. Not semantically meaningful — just stable,
/// which is all the index and search tests need.
pub struct FakeEmbedder {
    dimensions: usize,
}

impl FakeEmbedder {
    pub fn new(dimensions: usize) -> FakeEmbedder {
        FakeEmbedder { dimensions }
    }
}

#[async_trait]
impl Embedder for FakeEmbedder {
    fn dimensions(&self) -> usize {
        self.dimensions
    }

    fn model_id(&self) -> &str {
        "fake-embedder"
    }

    async fn embed(&self, inputs: &[String]) -> Result<Vec<Embedding>, ModelError> {
        Ok(inputs
            .iter()
            .map(|text| embed_one(text, self.dimensions))
            .collect())
    }
}

/// Fill a vector from a PRNG seeded by a stable hash of the text, then L2-normalize it.
fn embed_one(text: &str, dimensions: usize) -> Embedding {
    let mut state = hash64(text);
    let mut vector = Vec::with_capacity(dimensions);
    for _ in 0..dimensions {
        // xorshift64.
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        let fraction = (state >> 11) as f64 / (1u64 << 53) as f64; // [0, 1)
        vector.push((fraction * 2.0 - 1.0) as f32); // [-1, 1)
    }
    normalize(&mut vector);
    vector
}

/// FNV-1a, deterministic and dependency-free. The low bit is forced on so the xorshift seed is
/// never zero (which would degenerate to the all-zero stream).
fn hash64(text: &str) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in text.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash | 1
}

fn normalize(vector: &mut [f32]) {
    let norm = vector.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        for component in vector.iter_mut() {
            *component /= norm;
        }
    }
}
