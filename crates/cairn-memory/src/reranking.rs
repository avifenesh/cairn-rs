//! Reranking strategies for retrieval results (RFC 003).
//!
//! MMR (Maximal Marginal Relevance) balances relevance against diversity
//! to reduce redundancy in retrieval results.

use crate::retrieval::RetrievalResult;

/// Apply MMR reranking to a set of scored results.
///
/// `lambda` controls the relevance-diversity tradeoff:
/// - 1.0 = pure relevance (no diversity penalty)
/// - 0.0 = pure diversity (ignores relevance)
/// - 0.5 = balanced (typical default)
///
/// Uses cosine similarity on embeddings when available, falls back to
/// word-overlap similarity on chunk text.
pub fn mmr_rerank(results: &[RetrievalResult], limit: usize, lambda: f64) -> Vec<RetrievalResult> {
    if results.is_empty() || limit == 0 {
        return vec![];
    }

    let n = results.len();
    let limit = limit.min(n);

    // Precompute pairwise similarity matrix.
    let sim_matrix = build_similarity_matrix(results);

    // Normalize relevance scores to [0, 1] for fair MMR comparison.
    let max_score = results
        .iter()
        .map(|r| r.score)
        .fold(f64::NEG_INFINITY, f64::max);
    let min_score = results
        .iter()
        .map(|r| r.score)
        .fold(f64::INFINITY, f64::min);
    let score_range = (max_score - min_score).max(1e-10);

    let norm_scores: Vec<f64> = results
        .iter()
        .map(|r| (r.score - min_score) / score_range)
        .collect();

    let mut selected: Vec<usize> = Vec::with_capacity(limit);
    let mut remaining: Vec<usize> = (0..n).collect();

    // Greedily select items by MMR criterion.
    for _ in 0..limit {
        if remaining.is_empty() {
            break;
        }

        let best_idx = remaining
            .iter()
            .copied()
            .max_by(|&a, &b| {
                let mmr_a = mmr_score(a, &selected, &norm_scores, &sim_matrix, lambda);
                let mmr_b = mmr_score(b, &selected, &norm_scores, &sim_matrix, lambda);
                mmr_a.partial_cmp(&mmr_b).unwrap_or(std::cmp::Ordering::Equal)
            })
            .unwrap();

        selected.push(best_idx);
        remaining.retain(|&i| i != best_idx);
    }

    selected
        .into_iter()
        .map(|i| results[i].clone())
        .collect()
}

/// MMR score for candidate `i` given already-selected items.
///
/// MMR(i) = λ * relevance(i) - (1-λ) * max_sim(i, selected)
fn mmr_score(
    i: usize,
    selected: &[usize],
    norm_scores: &[f64],
    sim_matrix: &[Vec<f64>],
    lambda: f64,
) -> f64 {
    let relevance = norm_scores[i];

    let max_sim = if selected.is_empty() {
        0.0
    } else {
        selected
            .iter()
            .map(|&j| sim_matrix[i][j])
            .fold(f64::NEG_INFINITY, f64::max)
    };

    lambda * relevance - (1.0 - lambda) * max_sim
}

/// Build an n×n similarity matrix between results.
///
/// Uses cosine similarity on embeddings when available for both items,
/// falls back to word-overlap (Jaccard) similarity on text.
fn build_similarity_matrix(results: &[RetrievalResult]) -> Vec<Vec<f64>> {
    let n = results.len();
    let mut matrix = vec![vec![0.0_f64; n]; n];

    for i in 0..n {
        matrix[i][i] = 1.0;
        for j in (i + 1)..n {
            let sim = pairwise_similarity(&results[i], &results[j]);
            matrix[i][j] = sim;
            matrix[j][i] = sim;
        }
    }

    matrix
}

/// Compute similarity between two results.
fn pairwise_similarity(a: &RetrievalResult, b: &RetrievalResult) -> f64 {
    // Prefer cosine similarity on embeddings when both are present.
    if let (Some(ref ea), Some(ref eb)) = (&a.chunk.embedding, &b.chunk.embedding) {
        if !ea.is_empty() && !eb.is_empty() {
            return cosine_similarity(ea, eb);
        }
    }

    // Fallback: word-overlap Jaccard similarity.
    text_jaccard(&a.chunk.text, &b.chunk.text)
}

/// Cosine similarity between two embedding vectors.
fn cosine_similarity(a: &[f32], b: &[f32]) -> f64 {
    let len = a.len().min(b.len());
    if len == 0 {
        return 0.0;
    }

    let mut dot = 0.0_f64;
    let mut norm_a = 0.0_f64;
    let mut norm_b = 0.0_f64;

    for i in 0..len {
        let va = a[i] as f64;
        let vb = b[i] as f64;
        dot += va * vb;
        norm_a += va * va;
        norm_b += vb * vb;
    }

    let denom = norm_a.sqrt() * norm_b.sqrt();
    if denom < 1e-10 {
        return 0.0;
    }

    (dot / denom).clamp(-1.0, 1.0)
}

/// Jaccard similarity on lowercased word sets.
fn text_jaccard(a: &str, b: &str) -> f64 {
    let a_lower = a.to_lowercase();
    let b_lower = b.to_lowercase();
    let set_a: std::collections::HashSet<&str> = a_lower.split_whitespace().collect();
    let set_b: std::collections::HashSet<&str> = b_lower.split_whitespace().collect();

    let intersection = set_a.intersection(&set_b).count();
    let union = set_a.union(&set_b).count();

    if union == 0 {
        return 0.0;
    }

    intersection as f64 / union as f64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ingest::{ChunkRecord, SourceType};
    use crate::retrieval::ScoringBreakdown;
    use cairn_domain::{ChunkId, KnowledgeDocumentId, ProjectKey, SourceId};

    fn make_result(id: &str, text: &str, score: f64, embedding: Option<Vec<f32>>) -> RetrievalResult {
        RetrievalResult {
            chunk: ChunkRecord {
                chunk_id: ChunkId::new(id),
                document_id: KnowledgeDocumentId::new("doc"),
                source_id: SourceId::new("src"),
                source_type: SourceType::PlainText,
                project: ProjectKey::new("t", "w", "p"),
                text: text.to_owned(),
                position: 0,
                created_at: 0,
                updated_at: None,
                provenance_metadata: None,
                credibility_score: None,
                graph_linkage: None,
                embedding,
                content_hash: None,
            },
            score,
            breakdown: ScoringBreakdown::default(),
        }
    }

    #[test]
    fn mmr_empty_input_returns_empty() {
        let results = mmr_rerank(&[], 5, 0.5);
        assert!(results.is_empty());
    }

    #[test]
    fn mmr_single_result_returns_it() {
        let input = vec![make_result("c1", "hello world", 1.0, None)];
        let results = mmr_rerank(&input, 5, 0.5);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].chunk.chunk_id.as_str(), "c1");
    }

    #[test]
    fn mmr_lambda_one_preserves_relevance_order() {
        let input = vec![
            make_result("c1", "rust programming", 0.9, None),
            make_result("c2", "rust programming language", 0.7, None),
            make_result("c3", "python scripting", 0.5, None),
        ];
        // lambda=1.0 means pure relevance, no diversity penalty.
        let results = mmr_rerank(&input, 3, 1.0);
        assert_eq!(results[0].chunk.chunk_id.as_str(), "c1");
        assert_eq!(results[1].chunk.chunk_id.as_str(), "c2");
        assert_eq!(results[2].chunk.chunk_id.as_str(), "c3");
    }

    #[test]
    fn mmr_promotes_diversity_with_low_lambda() {
        // Two very similar results and one different one.
        let input = vec![
            make_result("c1", "rust ownership borrowing", 0.9, None),
            make_result("c2", "rust ownership borrowing rules", 0.85, None),
            make_result("c3", "python dynamic typing", 0.5, None),
        ];
        // lambda=0.3 heavily favors diversity.
        let results = mmr_rerank(&input, 3, 0.3);
        // First pick is still highest relevance.
        assert_eq!(results[0].chunk.chunk_id.as_str(), "c1");
        // Second pick should be the diverse one (python) since c2 is very similar to c1.
        assert_eq!(
            results[1].chunk.chunk_id.as_str(),
            "c3",
            "MMR with low lambda should promote diverse results over similar ones"
        );
    }

    #[test]
    fn mmr_with_embeddings_uses_cosine() {
        // c1 and c2 have near-identical embeddings, c3 is orthogonal.
        let input = vec![
            make_result("c1", "alpha", 0.9, Some(vec![1.0, 0.0, 0.0])),
            make_result("c2", "beta", 0.85, Some(vec![0.99, 0.1, 0.0])),
            make_result("c3", "gamma", 0.5, Some(vec![0.0, 0.0, 1.0])),
        ];
        let results = mmr_rerank(&input, 3, 0.3);
        assert_eq!(results[0].chunk.chunk_id.as_str(), "c1");
        // c3 is orthogonal to c1 so should be preferred over c2 which is nearly parallel.
        assert_eq!(
            results[1].chunk.chunk_id.as_str(),
            "c3",
            "MMR should prefer orthogonal embedding over near-duplicate"
        );
    }

    #[test]
    fn mmr_respects_limit() {
        let input = vec![
            make_result("c1", "one", 0.9, None),
            make_result("c2", "two", 0.7, None),
            make_result("c3", "three", 0.5, None),
        ];
        let results = mmr_rerank(&input, 2, 0.5);
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn cosine_similarity_identical_vectors() {
        let a = vec![1.0, 2.0, 3.0];
        let sim = cosine_similarity(&a, &a);
        assert!((sim - 1.0).abs() < 1e-6);
    }

    #[test]
    fn cosine_similarity_orthogonal_vectors() {
        let a = vec![1.0, 0.0];
        let b = vec![0.0, 1.0];
        let sim = cosine_similarity(&a, &b);
        assert!(sim.abs() < 1e-6);
    }

    #[test]
    fn text_jaccard_identical_strings() {
        let sim = text_jaccard("hello world", "hello world");
        assert!((sim - 1.0).abs() < 1e-6);
    }

    #[test]
    fn text_jaccard_disjoint_strings() {
        let sim = text_jaccard("hello world", "foo bar");
        assert!(sim.abs() < 1e-6);
    }
}
