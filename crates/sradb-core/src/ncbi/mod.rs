//! Wrappers for NCBI eUtils endpoints.

pub mod efetch;
pub mod esearch;
pub mod esummary;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct EsearchResult {
    pub count: u64,
    pub webenv: String,
    pub query_key: String,
    pub ids: Vec<String>,
}
