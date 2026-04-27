//! CNCB-NGDC GSA/INSDC browse page helpers.

use std::sync::LazyLock;

use regex::Regex;

use crate::error::{Result, SradbError};
use crate::http::{HttpClient, Service};

static HTTP_DOWNLOAD_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"https://download2\.cncb\.ac\.cn/[^\s"<>]+"#).expect("valid NGDC HTTP regex")
});
static FTP_DOWNLOAD_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"ftp://download2\.cncb\.ac\.cn/[^\s"<>]+"#).expect("valid NGDC FTP regex")
});

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct NgdcDownloadLinks {
    pub http: Option<String>,
    pub ftp: Option<String>,
}

#[must_use]
pub fn browse_url(submission_accession: &str, run_accession: &str) -> String {
    format!("https://ngdc.cncb.ac.cn/gsa/browse/insdc/{submission_accession}/{run_accession}")
}

pub async fn fetch_download_links(
    http: &HttpClient,
    submission_accession: &str,
    run_accession: &str,
) -> Result<NgdcDownloadLinks> {
    let url = browse_url(submission_accession, run_accession);
    let html = http
        .get_text("ngdc_browse", Service::Other, &url, &[])
        .await?;
    let links = parse_download_links(&html);
    if links.http.is_none() && links.ftp.is_none() {
        return Err(SradbError::Parse {
            endpoint: "ngdc_browse",
            message: format!("no NGDC download links found at {url}"),
        });
    }
    Ok(links)
}

#[must_use]
pub fn parse_download_links(html: &str) -> NgdcDownloadLinks {
    NgdcDownloadLinks {
        http: HTTP_DOWNLOAD_RE
            .find(html)
            .map(|m| normalize_url(m.as_str())),
        ftp: FTP_DOWNLOAD_RE
            .find(html)
            .map(|m| normalize_url(m.as_str())),
    }
}

fn normalize_url(url: &str) -> String {
    let Some((scheme, rest)) = url.split_once("://") else {
        return url.to_owned();
    };
    let mut rest = rest.to_owned();
    while rest.contains("//") {
        rest = rest.replace("//", "/");
    }
    format!("{scheme}://{rest}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn browse_url_uses_submission_and_run_accessions() {
        assert_eq!(
            browse_url("SRA1656025", "SRR24921613"),
            "https://ngdc.cncb.ac.cn/gsa/browse/insdc/SRA1656025/SRR24921613"
        );
    }

    #[test]
    fn parses_insdc_http_and_ftp_links() {
        let html = r#"
            <strong>Http: </strong>
            <a href="https://download2.cncb.ac.cn/INSDC/SRA/8/SRR8361/SRR8361601//SRR8361601">
              https://download2.cncb.ac.cn/INSDC/SRA/8/SRR8361/SRR8361601/SRR8361601
            </a>
            <strong>Ftp: </strong>
            <span>ftp://download2.cncb.ac.cn/INSDC/SRA/8/SRR8361/SRR8361601/SRR8361601</span>
        "#;
        let links = parse_download_links(html);
        assert_eq!(
            links.http.as_deref(),
            Some("https://download2.cncb.ac.cn/INSDC/SRA/8/SRR8361/SRR8361601/SRR8361601")
        );
        assert_eq!(
            links.ftp.as_deref(),
            Some("ftp://download2.cncb.ac.cn/INSDC/SRA/8/SRR8361/SRR8361601/SRR8361601")
        );
    }

    #[test]
    fn parses_insdc3_links_with_sra_suffix() {
        let html = r#"
            <a href="https://download2.cncb.ac.cn/INSDC3/SRA/24/SRR24921/SRR24921613//SRR24921613.sra">
              https://download2.cncb.ac.cn/INSDC3/SRA/24/SRR24921/SRR24921613/SRR24921613.sra
            </a>
            <span>ftp://download2.cncb.ac.cn/INSDC3/SRA/24/SRR24921/SRR24921613/SRR24921613.sra</span>
        "#;
        let links = parse_download_links(html);
        assert_eq!(
            links.http.as_deref(),
            Some("https://download2.cncb.ac.cn/INSDC3/SRA/24/SRR24921/SRR24921613/SRR24921613.sra")
        );
        assert_eq!(
            links.ftp.as_deref(),
            Some("ftp://download2.cncb.ac.cn/INSDC3/SRA/24/SRR24921/SRR24921613/SRR24921613.sra")
        );
    }
}
