//! Bridge GPUI image loads onto Tokio; credentials stay in the forge adapter.
use crate::backend::Executor;
use futures_util::future::BoxFuture;
use gpui::http_client::{AsyncBody, HttpClient, Method, Request, Response, Url, http::HeaderValue};
use mrde_core::config::Config;
use std::sync::{Arc, Mutex};
static REQUESTED: Mutex<Vec<String>> = Mutex::new(Vec::new());
/// GPUI caches images by URL, so a different project's credentials must not reuse them.
pub fn clear_cache(cx: &mut gpui::App) {
    for url in REQUESTED
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .drain(..)
    {
        cx.remove_asset::<gpui::ImgResourceLoader>(&gpui::Resource::Uri(url.into()));
    }
}

pub struct ImageHttpClient {
    runtime: Arc<tokio::runtime::Runtime>,
    executor: Arc<dyn Executor>,
    config: Config,
    permits: Arc<tokio::sync::Semaphore>,
}
impl ImageHttpClient {
    pub fn new(
        runtime: Arc<tokio::runtime::Runtime>,
        executor: Arc<dyn Executor>,
        config: Config,
    ) -> Self {
        Self {
            runtime,
            executor,
            config,
            permits: Arc::new(tokio::sync::Semaphore::new(4)),
        }
    }
}
impl HttpClient for ImageHttpClient {
    fn type_name(&self) -> &'static str {
        "mrde::ImageHttpClient"
    }
    fn user_agent(&self) -> Option<&HeaderValue> {
        None
    }
    fn proxy(&self) -> Option<&Url> {
        None
    }
    fn send(
        &self,
        request: Request<AsyncBody>,
    ) -> BoxFuture<'static, anyhow::Result<Response<AsyncBody>>> {
        if request.method() != Method::GET {
            return Box::pin(async { anyhow::bail!("The image client supports GET only") });
        }
        let url = request.uri().to_string();
        {
            let mut requested = REQUESTED.lock().unwrap_or_else(|e| e.into_inner());
            if !requested.contains(&url) {
                requested.push(url.clone());
            }
        }
        let executor = self.executor.clone();
        let config = self.config.clone();
        let permits = self.permits.clone();
        let task = self.runtime.spawn(async move {
            let _permit = permits.acquire_owned().await?;
            executor.download_image(config, url).await
        });
        Box::pin(async move {
            let bytes = task.await??;
            Ok(Response::builder()
                .status(200)
                .body(AsyncBody::from(bytes))?)
        })
    }
}
