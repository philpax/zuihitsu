// Embed two strings and report their cosine similarity — a debug utility for tuning the dedup
// and consolidation similarity thresholds.

use zuihitsu::{Embedder, OpenAiEmbedder, config::EnvConfig};

use crate::cli::error::CliError;

/// Embed `a` and `b` through the configured embedding endpoint and print their cosine similarity,
/// alongside whether the score would trigger the dedup (0.95) or consolidation (0.85) thresholds.
pub(crate) fn embed(config: &EnvConfig, a: &str, b: &str) -> Result<(), CliError> {
    let embedder = OpenAiEmbedder::new(&config.embedding);
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|source| {
            CliError::Embed(format!("could not start the async runtime: {source}"))
        })?;
    let embeddings = runtime
        .block_on(embedder.embed(&[a.to_owned(), b.to_owned()]))
        .map_err(|e| CliError::Embed(format!("embedding failed: {e}")))?;

    let emb_a = &embeddings[0];
    let emb_b = &embeddings[1];
    let similarity: f32 = emb_a.iter().zip(emb_b).map(|(x, y)| x * y).sum();

    println!("text A: {a:?}");
    println!("text B: {b:?}");
    println!("cosine similarity: {similarity:.4}");
    println!();
    println!(
        "dedup threshold (default 0.95): {}",
        if similarity >= 0.95 {
            "ABOVE — would be rejected as a duplicate"
        } else {
            "BELOW — would not be rejected"
        }
    );
    println!(
        "consolidation threshold (default 0.85): {}",
        if similarity >= 0.85 {
            "ABOVE — would be clustered for consolidation"
        } else {
            "BELOW — would not be clustered"
        }
    );
    Ok(())
}
