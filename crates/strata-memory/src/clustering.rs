use serde::{Deserialize, Serialize};

use crate::embedding::cosine_similarity;

/// A point in vector space for clustering.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VectorPoint<T> {
    pub id: String,
    pub embedding: Vec<f32>,
    pub payload: T,
}

/// A cluster of vector points with an elected medoid.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpatialCluster<T> {
    pub cluster_id: usize,
    pub centroid: Vec<f32>,
    pub medoid_index: usize,
    pub medoid_id: String,
    pub points: Vec<VectorPoint<T>>,
}

/// Spatial vector clustering engine implementing DBSCAN-style distance grouping and Medoid election.
pub struct SpatialClusterer {
    /// Cosine distance threshold (1.0 - cosine_sim) to consider two points neighbors (default 0.20 -> sim >= 0.80)
    pub distance_threshold: f32,
    /// Minimum points to form a dense cluster (default 2)
    pub min_cluster_size: usize,
}

impl Default for SpatialClusterer {
    fn default() -> Self {
        Self {
            distance_threshold: 0.20,
            min_cluster_size: 2,
        }
    }
}

impl SpatialClusterer {
    pub fn new(distance_threshold: f32, min_cluster_size: usize) -> Self {
        Self {
            distance_threshold,
            min_cluster_size,
        }
    }

    /// Calculate cosine distance: 1.0 - cosine_similarity(a, b).
    pub fn distance(a: &[f32], b: &[f32]) -> f32 {
        let sim = cosine_similarity(a, b);
        (1.0 - sim).max(0.0)
    }

    /// Elect the medoid of a list of points (the point with minimal sum of distances to all other points).
    pub fn find_medoid<T>(points: &[VectorPoint<T>]) -> usize {
        if points.is_empty() {
            return 0;
        }
        if points.len() == 1 {
            return 0;
        }

        let mut best_idx = 0;
        let mut min_total_distance = f32::MAX;

        for (i, p_i) in points.iter().enumerate() {
            let mut total_dist = 0.0f32;
            for (j, p_j) in points.iter().enumerate() {
                if i != j {
                    total_dist += Self::distance(&p_i.embedding, &p_j.embedding);
                }
            }
            if total_dist < min_total_distance {
                min_total_distance = total_dist;
                best_idx = i;
            }
        }

        best_idx
    }

    /// Cluster points using a deterministic single-pass leader / nearest-centroid grouping.
    pub fn cluster<T: Clone>(&self, points: Vec<VectorPoint<T>>) -> Vec<SpatialCluster<T>> {
        if points.is_empty() {
            return Vec::new();
        }

        let mut clusters: Vec<Vec<VectorPoint<T>>> = Vec::new();

        for point in points {
            let mut best_cluster_idx = None;
            let mut min_dist = self.distance_threshold;

            for (c_idx, c_points) in clusters.iter().enumerate() {
                // Check distance to cluster centroid or first point
                let dist = Self::distance(&point.embedding, &c_points[0].embedding);
                if dist < min_dist {
                    min_dist = dist;
                    best_cluster_idx = Some(c_idx);
                }
            }

            if let Some(c_idx) = best_cluster_idx {
                clusters[c_idx].push(point);
            } else {
                clusters.push(vec![point]);
            }
        }

        let mut result = Vec::new();
        for (idx, c_points) in clusters.into_iter().enumerate() {
            let medoid_idx = Self::find_medoid(&c_points);
            let medoid_id = c_points[medoid_idx].id.clone();

            // Compute centroid
            let dim = c_points[0].embedding.len();
            let mut centroid = vec![0.0f32; dim];
            let n = c_points.len() as f32;
            for pt in &c_points {
                for (i, v) in pt.embedding.iter().enumerate() {
                    if i < dim {
                        centroid[i] += v / n;
                    }
                }
            }

            result.push(SpatialCluster {
                cluster_id: idx + 1,
                centroid,
                medoid_index: medoid_idx,
                medoid_id,
                points: c_points,
            });
        }

        result
    }
}
