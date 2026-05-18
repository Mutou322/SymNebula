//! Network abstraction for inter-cluster communication.
//!
//! Provides a ring-topology in-memory communicator built on `tokio::sync::mpsc`
//! channels.  Each cluster sends boundary-node updates to its clockwise
//! neighbour and receives from its counter-clockwise neighbour.

use tokio::sync::mpsc;

/// A message carrying updated values for boundary nodes shared between
/// neighbouring clusters.
#[derive(Debug, Clone, PartialEq)]
pub struct BoundaryMessage {
    /// Index of the sending cluster.
    pub from_cluster: usize,
    /// Index of the destination cluster.
    pub to_cluster: usize,
    /// Pairs of `(node_id, new_value)` for each boundary node that changed.
    pub node_updates: Vec<(u32, f32)>,
}

/// Ring-topology communicator backed by `mpsc` channels.
///
/// # Topology
///
/// Cluster `i` sends messages via `senders[i]`; those messages arrive at
/// `receivers[(i + 1) % n]`, i.e. at the clockwise neighbour.  Conversely,
/// cluster `i` reads from `receivers[i]`, which carries messages sent by
/// cluster `(i + n - 1) % n` (the counter-clockwise neighbour).
///
/// # Buffer capacity
///
/// Each channel is created with a capacity of 64 messages.  If the receiver
/// falls behind, `send` will asynchronously wait for buffer space.
pub struct InMemoryCommunicator {
    /// Sender handles, indexed by source cluster.
    pub senders: Vec<mpsc::Sender<BoundaryMessage>>,
    /// Receiver handles, indexed by destination cluster.
    pub receivers: Vec<mpsc::Receiver<BoundaryMessage>>,
    /// Total number of clusters in the ring.
    pub n_clusters: usize,
}

impl InMemoryCommunicator {
    /// Create a ring communicator for `n` clusters.
    ///
    /// Constructs `n` `mpsc` channels.  Channel `i` connects `senders[i]`
    /// (used by cluster `i`) to `receivers[(i + 1) % n]` (read by cluster
    /// `(i + 1) % n`).
    pub fn new(n: usize) -> Self {
        // Allocate n channels and pair them according to the ring topology:
        //   channel[i] → senders[i]  → receivers[(i + 1) % n]
        let mut senders: Vec<mpsc::Sender<BoundaryMessage>> = Vec::with_capacity(n);
        let mut receivers: Vec<Option<mpsc::Receiver<BoundaryMessage>>> =
            (0..n).map(|_| None).collect();

        for i in 0..n {
            let (tx, rx) = mpsc::channel(64);
            senders.push(tx);
            // The receiver half of channel i is placed at the clockwise neighbour.
            receivers[(i + 1) % n] = Some(rx);
        }

        Self {
            senders,
            receivers: receivers.into_iter().map(|opt| opt.unwrap()).collect(),
            n_clusters: n,
        }
    }

    /// Send a message from cluster `from` to its clockwise neighbour.
    ///
    /// The destination is determined by the ring topology at construction
    /// time -- the caller does not need to compute it.
    pub async fn send(&mut self, from: usize, msg: BoundaryMessage) {
        // `mpsc::Sender::send` waits for buffer capacity; ignore
        // `SendError` which only occurs when the receiver half is dropped.
        let _ = self.senders[from].send(msg).await;
    }

    /// Receive a message addressed to cluster `to` (sent by its
    /// counter-clockwise neighbour).
    ///
    /// Returns `None` when all senders feeding this receiver have been
    /// dropped (i.e. the channel is closed).
    pub async fn recv(&mut self, to: usize) -> Option<BoundaryMessage> {
        self.receivers[to].recv().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn ring_forward_single_message() {
        let mut comm = InMemoryCommunicator::new(3);

        let msg = BoundaryMessage {
            from_cluster: 0,
            to_cluster: 1,
            node_updates: vec![(7, 3.14)],
        };

        // Cluster 0 sends → cluster 1 receives
        comm.send(0, msg.clone()).await;
        let received = comm.recv(1).await;
        assert!(received.is_some());
        let r = received.unwrap();
        assert_eq!(r.from_cluster, 0);
        assert_eq!(r.to_cluster, 1);
        assert_eq!(r.node_updates.len(), 1);
        assert!((r.node_updates[0].1 - 3.14).abs() < 1e-6);
    }

    #[tokio::test]
    async fn ring_wraparound() {
        let mut comm = InMemoryCommunicator::new(3);

        // Cluster 2 (last) → Cluster 0 (first) via wraparound
        let msg = BoundaryMessage {
            from_cluster: 2,
            to_cluster: 0,
            node_updates: vec![(42, 0.0)],
        };
        comm.send(2, msg).await;
        let received = comm.recv(0).await;
        assert!(received.is_some());
        assert_eq!(received.unwrap().from_cluster, 2);
    }

    #[tokio::test]
    async fn full_ring_round_trip() {
        let mut comm = InMemoryCommunicator::new(4);

        // Each cluster sends one message clockwise
        for i in 0..4 {
            comm.send(
                i,
                BoundaryMessage {
                    from_cluster: i,
                    to_cluster: (i + 1) % 4,
                    node_updates: vec![(i as u32, i as f32)],
                },
            )
            .await;
        }

        // Every cluster should receive exactly one message
        for i in 0..4 {
            let r = comm.recv(i).await;
            assert!(r.is_some(), "cluster {} should have received a message", i);
            let msg = r.unwrap();
            // The sender is the counter-clockwise neighbour
            let expected_sender = (i + 3) % 4;
            assert_eq!(msg.from_cluster, expected_sender);
        }
    }

    #[tokio::test]
    async fn recv_returns_none_when_channel_closed() {
        // Standalone test: when all Sender clones are dropped, recv returns None.
        let (tx, mut rx) = mpsc::channel::<BoundaryMessage>(1);
        drop(tx);
        assert_eq!(rx.recv().await, None);
    }
}
