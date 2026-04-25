//! Stub. Filled in Task 5.

#[derive(Debug, thiserror::Error)]
pub enum SradbError {
    #[error("placeholder")]
    Placeholder,
}

pub type Result<T> = std::result::Result<T, SradbError>;
