//! Stub. Filled in Task 7.

#[derive(Debug, Default, Clone)]
pub struct ClientConfig {}

#[derive(Debug, Clone)]
pub struct SraClient {}

impl SraClient {
    #[must_use]
    pub fn new() -> Self { Self {} }
}

impl Default for SraClient { fn default() -> Self { Self::new() } }
