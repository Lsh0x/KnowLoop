//! Validation for DecisionVector and other neural routing types.

use crate::error::{NeuralRoutingError, Result};

/// Expected dimensionality for all embeddings in the neural routing system.
pub const EMBEDDING_DIM: usize = 256;

/// Tolerance for L2 norm check (||v|| should be ~1.0).
const L2_NORM_TOLERANCE: f32 = 0.05;

/// Validate a decision/query embedding vector.
///
/// Checks:
/// 1. Exactly 256 dimensions
/// 2. All values in [-1.0, 1.0]
/// 3. L2-normalized (norm ~= 1.0 within tolerance)
/// 4. No NaN or Infinity values
pub fn validate_embedding(embedding: &[f32]) -> Result<()> {
    // Check dimensionality
    if embedding.len() != EMBEDDING_DIM {
        return Err(NeuralRoutingError::InvalidVector(format!(
            "expected {} dimensions, got {}",
            EMBEDDING_DIM,
            embedding.len()
        )));
    }

    // Check for NaN/Inf and bounds
    let mut l2_sum = 0.0f64;
    for (i, &v) in embedding.iter().enumerate() {
        if v.is_nan() || v.is_infinite() {
            return Err(NeuralRoutingError::InvalidVector(format!(
                "dimension {} contains NaN or Infinity",
                i
            )));
        }
        if v < -1.0 || v > 1.0 {
            return Err(NeuralRoutingError::InvalidVector(format!(
                "dimension {} value {} is out of bounds [-1.0, 1.0]",
                i, v
            )));
        }
        l2_sum += (v as f64) * (v as f64);
    }

    // Check L2 normalization
    let l2_norm = l2_sum.sqrt() as f32;
    if (l2_norm - 1.0).abs() > L2_NORM_TOLERANCE {
        return Err(NeuralRoutingError::InvalidVector(format!(
            "L2 norm is {:.4}, expected ~1.0 (tolerance {})",
            l2_norm, L2_NORM_TOLERANCE
        )));
    }

    Ok(())
}

/// Normalize a vector to unit L2 norm in-place.
pub fn l2_normalize(embedding: &mut [f32]) {
    let norm: f64 = embedding.iter().map(|&v| (v as f64) * (v as f64)).sum::<f64>().sqrt();
    if norm > 1e-10 {
        let inv_norm = 1.0 / norm as f32;
        for v in embedding.iter_mut() {
            *v *= inv_norm;
        }
    }
}

/// Compute cosine similarity between two vectors.
/// Assumes both are L2-normalized, so cosine = dot product.
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f64 {
    debug_assert_eq!(a.len(), b.len(), "vectors must have same dimensions");
    a.iter()
        .zip(b.iter())
        .map(|(&x, &y)| (x as f64) * (y as f64))
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_unit_vector(dim: usize) -> Vec<f32> {
        let val = 1.0 / (dim as f32).sqrt();
        vec![val; dim]
    }

    #[test]
    fn test_valid_embedding() {
        let v = make_unit_vector(EMBEDDING_DIM);
        assert!(validate_embedding(&v).is_ok());
    }

    #[test]
    fn test_wrong_dimension() {
        let v = make_unit_vector(128);
        let err = validate_embedding(&v).unwrap_err();
        assert!(err.to_string().contains("expected 256 dimensions, got 128"));
    }

    #[test]
    fn test_unnormalized_vector() {
        let v = vec![0.5; EMBEDDING_DIM]; // norm = sqrt(256 * 0.25) = 8.0
        let err = validate_embedding(&v).unwrap_err();
        assert!(err.to_string().contains("L2 norm is"));
    }

    #[test]
    fn test_nan_value() {
        let mut v = make_unit_vector(EMBEDDING_DIM);
        v[10] = f32::NAN;
        let err = validate_embedding(&v).unwrap_err();
        assert!(err.to_string().contains("NaN or Infinity"));
    }

    #[test]
    fn test_out_of_bounds() {
        let mut v = make_unit_vector(EMBEDDING_DIM);
        v[0] = 1.5;
        let err = validate_embedding(&v).unwrap_err();
        assert!(err.to_string().contains("out of bounds"));
    }

    #[test]
    fn test_l2_normalize() {
        let mut v = vec![3.0, 4.0];
        l2_normalize(&mut v);
        assert!((v[0] - 0.6).abs() < 1e-5);
        assert!((v[1] - 0.8).abs() < 1e-5);
    }

    #[test]
    fn test_cosine_similarity_identical() {
        let v = make_unit_vector(EMBEDDING_DIM);
        let sim = cosine_similarity(&v, &v);
        assert!((sim - 1.0).abs() < 1e-5);
    }

    #[test]
    fn test_cosine_similarity_orthogonal() {
        let mut a = vec![0.0; EMBEDDING_DIM];
        let mut b = vec![0.0; EMBEDDING_DIM];
        a[0] = 1.0;
        b[1] = 1.0;
        let sim = cosine_similarity(&a, &b);
        assert!(sim.abs() < 1e-5);
    }
}
