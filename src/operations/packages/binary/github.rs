use serde::Deserialize;

#[derive(Deserialize)]
pub(crate) struct Release {
    pub assets: Vec<Asset>,
}

#[derive(Deserialize)]
pub(crate) struct Asset {
    pub name: String,
    pub browser_download_url: String,
}
