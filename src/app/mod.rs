pub mod upload;
pub mod download;

pub fn register_urls(cfg: &mut actix_web::web::ServiceConfig) {
    upload::urls::register_urls(cfg);
    download::urls::register_urls(cfg);
}
