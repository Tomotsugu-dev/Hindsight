//! 测试用的假网络和图片。

use std::collections::HashMap;
use std::io::Cursor;
use std::sync::Mutex;

use image::ImageFormat;
use reqwest::Url;

use super::{Fetched, Http};

/// 按网址返回事先放好的回答，并记下请求过哪些网址。
#[derive(Default)]
pub(in crate::icons::site) struct FakeHttp {
    /// 网址 → (跳转后的网址, 状态码, 内容)
    answers: HashMap<String, (String, u16, Vec<u8>)>,
    requested: Mutex<Vec<String>>,
}

impl FakeHttp {
    pub(in crate::icons::site) fn answer(
        mut self,
        url: &str,
        status: u16,
        body: impl Into<Vec<u8>>,
    ) -> Self {
        self.answers
            .insert(url.into(), (url.into(), status, body.into()));
        self
    }

    /// `url` 跳到 `to`，回答的是 `to` 的内容。
    pub(in crate::icons::site) fn redirect(
        mut self,
        url: &str,
        to: &str,
        status: u16,
        body: impl Into<Vec<u8>>,
    ) -> Self {
        self.answers
            .insert(url.into(), (to.into(), status, body.into()));
        self
    }

    pub(in crate::icons::site) fn requested(&self) -> Vec<String> {
        self.requested.lock().unwrap().clone()
    }
}

impl Http for FakeHttp {
    async fn get(&self, url: Url) -> Option<Fetched> {
        self.requested.lock().unwrap().push(url.to_string());
        let (to, status, body) = self.answers.get(url.as_str())?.clone();
        Some(Fetched {
            url: Url::parse(&to).unwrap(),
            status,
            body,
        })
    }
}

pub(in crate::icons::site) fn png(px: u32) -> Vec<u8> {
    encode(px, ImageFormat::Png)
}

pub(in crate::icons::site) fn ico(px: u32) -> Vec<u8> {
    encode(px, ImageFormat::Ico)
}

fn encode(px: u32, format: ImageFormat) -> Vec<u8> {
    let img = image::DynamicImage::new_rgba8(px, px);
    let mut out = Vec::new();
    img.write_to(&mut Cursor::new(&mut out), format).unwrap();
    out
}
