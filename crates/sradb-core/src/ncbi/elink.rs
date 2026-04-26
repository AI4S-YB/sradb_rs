//! NCBI elink wrapper: link UIDs across databases (e.g. pubmed → pmc).

use serde::Deserialize;

use crate::error::Result;
use crate::http::{HttpClient, Service};

const CONTEXT: &str = "elink";

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct Envelope {
    linksets: Vec<LinkSet>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct LinkSet {
    linksetdbs: Vec<LinkSetDb>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct LinkSetDb {
    linkname: String,
    links: Vec<String>,
}

/// Link a `PubMed` ID to PMC IDs. Returns the PMC numeric IDs (without "PMC" prefix).
pub async fn pmid_to_pmc_ids(
    http: &HttpClient,
    base_url: &str,
    pmid: u64,
    api_key: Option<&str>,
) -> Result<Vec<String>> {
    let url = format!("{base_url}/elink.fcgi");
    let pmid_s = pmid.to_string();
    let mut q: Vec<(&str, &str)> = vec![
        ("dbfrom", "pubmed"),
        ("db", "pmc"),
        ("id", &pmid_s),
        ("retmode", "json"),
    ];
    if let Some(k) = api_key {
        q.push(("api_key", k));
    }
    let env: Envelope = http.get_json(CONTEXT, Service::Ncbi, &url, &q).await?;
    let mut out: Vec<String> = Vec::new();
    for ls in &env.linksets {
        for db in &ls.linksetdbs {
            if db.linkname == "pubmed_pmc" {
                out.extend(db.links.iter().cloned());
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use wiremock::matchers::{method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn pmid_to_pmc_ids_extracts_link() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/elink.fcgi"))
            .and(query_param("dbfrom", "pubmed"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"{"linksets":[{"dbfrom":"pubmed","ids":["39528918"],"linksetdbs":[{"dbto":"pmc","linkname":"pubmed_pmc","links":["10802650"]}]}]}"#,
            ))
            .mount(&server)
            .await;

        let http = HttpClient::new(10, 10, 0, Duration::from_secs(5)).unwrap();
        let pmcs = pmid_to_pmc_ids(&http, &server.uri(), 39_528_918, None)
            .await
            .unwrap();
        assert_eq!(pmcs, vec!["10802650".to_string()]);
    }

    #[tokio::test]
    async fn pmid_with_no_pmc_link_returns_empty() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/elink.fcgi"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"{"linksets":[{"dbfrom":"pubmed","ids":["1"],"linksetdbs":[]}]}"#,
            ))
            .mount(&server)
            .await;

        let http = HttpClient::new(10, 10, 0, Duration::from_secs(5)).unwrap();
        let pmcs = pmid_to_pmc_ids(&http, &server.uri(), 1, None)
            .await
            .unwrap();
        assert!(pmcs.is_empty());
    }
}
