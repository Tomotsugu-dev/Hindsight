//! Download addresses for models, the AI engine and OCR components.

use serde::{Deserialize, Serialize};
use url::Url;

use crate::error::{Error, Result};

pub const HUGGINGFACE_MIRROR: &str = "https://hf-mirror.com";
pub const GITHUB_PROXY: &str = "https://gh-proxy.org";
pub const NUGET_MIRROR: &str =
    "https://mirrors.huaweicloud.com/artifactory/api/nuget/v3/nuget-remote";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DownloadSourceMode {
    Mirror,
    Custom,
    #[default]
    #[serde(other)]
    Official,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DownloadSources {
    pub mode: DownloadSourceMode,
    pub huggingface_url: String,
    pub github_proxy_url: String,
    pub nuget_url: String,
}

impl Default for DownloadSources {
    fn default() -> Self {
        Self {
            mode: DownloadSourceMode::Official,
            huggingface_url: HUGGINGFACE_MIRROR.into(),
            github_proxy_url: GITHUB_PROXY.into(),
            nuget_url: NUGET_MIRROR.into(),
        }
    }
}

impl DownloadSources {
    pub fn validate(&self) -> Result<()> {
        if self.mode == DownloadSourceMode::Custom {
            mirror_base(&self.huggingface_url)?;
            mirror_base(&self.github_proxy_url)?;
            mirror_base(&self.nuget_url)?;
        }
        Ok(())
    }

    /// Rewrites the first request only; redirects and Range requests use the existing downloader.
    pub fn url(&self, original: &str) -> Result<String> {
        if self.mode == DownloadSourceMode::Official {
            return Ok(original.into());
        }
        let parsed =
            Url::parse(original).map_err(|_| Error::InvalidInput("Invalid download URL"))?;
        match parsed.host_str() {
            Some("huggingface.co") => append_path(
                if self.mode == DownloadSourceMode::Mirror {
                    HUGGINGFACE_MIRROR
                } else {
                    &self.huggingface_url
                },
                parsed.path(),
                parsed.query(),
            ),
            Some("github.com" | "raw.githubusercontent.com") => {
                let base = mirror_base(if self.mode == DownloadSourceMode::Mirror {
                    GITHUB_PROXY
                } else {
                    &self.github_proxy_url
                })?;
                Ok(format!(
                    "{}/{original}",
                    base.as_str().trim_end_matches('/')
                ))
            }
            Some("api.nuget.org") if parsed.path().starts_with("/v3-flatcontainer/") => {
                append_path(
                    if self.mode == DownloadSourceMode::Mirror {
                        NUGET_MIRROR
                    } else {
                        &self.nuget_url
                    },
                    parsed.path().trim_start_matches("/v3-flatcontainer/"),
                    parsed.query(),
                )
            }
            _ => Ok(original.into()),
        }
    }
}

fn append_path(base: &str, path: &str, query: Option<&str>) -> Result<String> {
    let mut target = mirror_base(base)?;
    let path = format!(
        "{}/{}",
        target.path().trim_end_matches('/'),
        path.trim_start_matches('/')
    );
    target.set_path(&path);
    target.set_query(query);
    Ok(target.into())
}

fn mirror_base(value: &str) -> Result<Url> {
    let url = Url::parse(value.trim())
        .map_err(|_| Error::InvalidInput("Download source must be an HTTP or HTTPS address"))?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(Error::InvalidInput(
            "Download source must be an HTTP or HTTPS address without credentials, query or fragment",
        ));
    }
    Ok(url)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn official_and_unrelated_addresses_are_preserved() {
        let hf = "https://huggingface.co/org/model/resolve/main/model.gguf?download=true";
        assert_eq!(DownloadSources::default().url(hf).unwrap(), hf);
        let mirrors = DownloadSources {
            mode: DownloadSourceMode::Mirror,
            ..Default::default()
        };
        for url in [
            "https://example.com/model.gguf",
            "https://huggingface.co.example.com/a",
        ] {
            assert_eq!(mirrors.url(url).unwrap(), url);
        }
    }

    #[test]
    fn preset_routes_models_release_assets_and_ocr_dictionary() {
        let mirrors = DownloadSources {
            mode: DownloadSourceMode::Mirror,
            ..Default::default()
        };
        assert_eq!(
            mirrors
                .url("https://huggingface.co/org/model/resolve/main/a%20b.gguf?download=true")
                .unwrap(),
            "https://hf-mirror.com/org/model/resolve/main/a%20b.gguf?download=true"
        );
        for url in [
            "https://github.com/ggml-org/llama.cpp/releases/download/b1/engine.zip",
            "https://github.com/microsoft/onnxruntime/releases/download/v1/runtime.zip",
            "https://raw.githubusercontent.com/PaddlePaddle/PaddleOCR/main/dict.txt",
        ] {
            assert_eq!(mirrors.url(url).unwrap(), format!("{GITHUB_PROXY}/{url}"));
        }
    }

    #[test]
    fn custom_sources_keep_base_paths_and_trim_whitespace() {
        let custom = DownloadSources {
            mode: DownloadSourceMode::Custom,
            huggingface_url: " https://mirror.example/hf/ ".into(),
            github_proxy_url: "https://mirror.example/github/".into(),
            nuget_url: "https://mirror.example/nuget/".into(),
        };
        custom.validate().unwrap();
        assert_eq!(
            custom
                .url("https://huggingface.co/org/repo/resolve/main/mmproj.gguf")
                .unwrap(),
            "https://mirror.example/hf/org/repo/resolve/main/mmproj.gguf"
        );
        assert_eq!(custom.url("https://github.com/org/repo/releases/download/v1/file.zip").unwrap(), "https://mirror.example/github/https://github.com/org/repo/releases/download/v1/file.zip");
    }

    #[test]
    fn invalid_custom_sources_are_rejected() {
        for address in [
            "",
            "not a URL",
            "file:///tmp/models",
            "https://user:pass@example.com",
            "https://example.com?key=value",
            "https://example.com#part",
        ] {
            let sources = DownloadSources {
                mode: DownloadSourceMode::Custom,
                huggingface_url: address.into(),
                ..Default::default()
            };
            assert!(sources.validate().is_err(), "{address}");
            assert!(sources
                .url("https://huggingface.co/org/model/resolve/main/model.gguf")
                .is_err());
        }
    }

    #[test]
    fn missing_and_future_modes_keep_official_downloads() {
        for json in ["{}", r#"{"mode":"futureSource"}"#] {
            let sources: DownloadSources = serde_json::from_str(json).unwrap();
            assert_eq!(sources.mode, DownloadSourceMode::Official);
        }
    }

    #[test]
    fn windows_ocr_packages_use_the_nuget_mirror() {
        let original = "https://api.nuget.org/v3-flatcontainer/microsoft.ai.directml/1.15.4/microsoft.ai.directml.1.15.4.nupkg";
        let mirrors = DownloadSources {
            mode: DownloadSourceMode::Mirror,
            ..Default::default()
        };
        assert_eq!(
            mirrors.url(original).unwrap(),
            format!(
                "{NUGET_MIRROR}/microsoft.ai.directml/1.15.4/microsoft.ai.directml.1.15.4.nupkg"
            )
        );
        let custom = DownloadSources {
            mode: DownloadSourceMode::Custom,
            nuget_url: "https://packages.example/flat".into(),
            ..Default::default()
        };
        assert_eq!(custom.url(original).unwrap(), "https://packages.example/flat/microsoft.ai.directml/1.15.4/microsoft.ai.directml.1.15.4.nupkg");
        assert_eq!(
            mirrors.url("https://api.nuget.org/v3/index.json").unwrap(),
            "https://api.nuget.org/v3/index.json"
        );
    }

    #[tokio::test]
    async fn configured_mirror_downloads_and_handles_both_resume_responses() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        for (partial, resumed) in [(false, false), (true, true), (true, false)] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let server = tokio::spawn(async move {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = Vec::new();
                loop {
                    let mut buffer = [0; 1024];
                    let n = socket.read(&mut buffer).await.unwrap();
                    assert!(n > 0);
                    request.extend_from_slice(&buffer[..n]);
                    if request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
                        break;
                    }
                }
                let (status, range, body) = if resumed {
                    (
                        "206 Partial Content",
                        "Content-Range: bytes 4-11/12\r\n",
                        "abcdefgh",
                    )
                } else {
                    ("200 OK", "", "GGUFabcdefgh")
                };
                let response = format!("HTTP/1.1 {status}\r\nContent-Length: {}\r\n{range}Connection: close\r\n\r\n{body}", body.len());
                socket.write_all(response.as_bytes()).await.unwrap();
                String::from_utf8(request).unwrap()
            });
            let dir = std::env::temp_dir()
                .join(format!("hindsight-model-mirror-{}", uuid::Uuid::new_v4()));
            tokio::fs::create_dir_all(&dir).await.unwrap();
            if partial {
                tokio::fs::write(dir.join("model.gguf.partial"), b"GGUF")
                    .await
                    .unwrap();
            }
            let cfg = crate::ai::config::AiConfig {
                models_path: dir.to_string_lossy().into_owned(),
                ..Default::default()
            };
            let sources = DownloadSources {
                mode: DownloadSourceMode::Custom,
                huggingface_url: format!("http://{address}"),
                ..Default::default()
            };
            let file = crate::ai::models::download_from_hf(
                &cfg,
                &sources,
                "org/model",
                "model.gguf",
                None,
                12,
                |_, _| {},
            )
            .await
            .unwrap();
            assert_eq!(tokio::fs::read(&file).await.unwrap(), b"GGUFabcdefgh");
            let request = server.await.unwrap().to_ascii_lowercase();
            assert!(request.starts_with("get /org/model/resolve/main/model.gguf http/1.1\r\n"));
            assert_eq!(request.contains("range: bytes=4-"), partial);
            assert!(!dir.join("model.gguf.partial").exists());
            tokio::fs::remove_dir_all(dir).await.unwrap();
        }
    }
}
