//! Message passing trait — the foundation for all GNN layers.

use candle_core::Tensor;

/// Trait for message passing layers in graph neural networks.
///
/// Implements the message-passing paradigm:
/// 1. `message()` — compute messages from neighbors
/// 2. `aggregate()` — aggregate messages (sum, mean, max)
/// 3. `update()` — update node representation
pub trait MessagePassing {
    /// Compute messages along edges.
    fn message(&self, x: &Tensor, edge_index: &Tensor, edge_type: Option<&Tensor>) -> candle_core::Result<Tensor>;

    /// Aggregate messages at each node.
    fn aggregate(&self, messages: &Tensor, edge_index: &Tensor, num_nodes: usize) -> candle_core::Result<Tensor>;

    /// Update node representations.
    fn update(&self, x: &Tensor, aggregated: &Tensor) -> candle_core::Result<Tensor>;

    /// Full forward pass: message -> aggregate -> update.
    fn forward(&self, x: &Tensor, edge_index: &Tensor, edge_type: Option<&Tensor>, num_nodes: usize) -> candle_core::Result<Tensor> {
        let messages = self.message(x, edge_index, edge_type)?;
        let aggregated = self.aggregate(&messages, edge_index, num_nodes)?;
        self.update(x, &aggregated)
    }
}
