use actix_web::web;

use super::views;

pub fn register_urls(cfg: &mut web::ServiceConfig) {
    cfg.route("/{tail:.*}", web::get().to(views::display_dir));
}
