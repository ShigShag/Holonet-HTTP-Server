use actix_web::{App, HttpServer, middleware::Logger, web};
use clap::Parser;
use env_logger::Env;
use std::path::{Path, PathBuf};
use tera::Tera;

mod app;
mod utils;

#[derive(Parser, Debug)]
#[command(
    version,
    about = "A simple upload server.\n\n\
Upload a file:\n\
curl -X POST -T file_path http://ip:port/upload\n\n\
Optional / Custom file and directory name with:\n\
curl -X POST -T file_path -H \"X-Target-File: desired_filename.ext\" -H \"X-Target-dir: dirname\" http://ip:port/upload"
)]
struct Args {
    /// Root directory
    #[arg(short, long, default_value = ".")]
    directory: String,

    /// Host to bind the server to
    #[arg(short = 'l', long, default_value = "0.0.0.0")]
    host: String,

    /// Port to host the server on
    #[arg(short, long, default_value_t = 7070)]
    port: u16,

    /// Use TLS encryption
    #[arg(long)]
    tls: bool,
}

pub struct State {
    pub base_path: PathBuf,
    pub tera: tera::Tera,
}

const DIR_LISTING_TEMPLATE_CONTENT: &str = include_str!("../static/templates/home.html");
const DIR_LISTING_TEMPLATE_NAME: &str = "home.html";

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    // Create logger
    env_logger::Builder::from_env(Env::default().default_filter_or("info")).init();

    let args = Args::parse();

    let server = HttpServer::new(move || {
        // Initialize Tera template engine
        let mut tera_instance = Tera::default();
        tera_instance
            .add_raw_template(DIR_LISTING_TEMPLATE_NAME, DIR_LISTING_TEMPLATE_CONTENT)
            .unwrap();

        let base_path: &Path = Path::new(&args.directory);
        let canonical_base_path: PathBuf = base_path.canonicalize().unwrap();

        let state = State {
            base_path: canonical_base_path,
            tera: tera_instance,
        };

        App::new()
            .wrap(Logger::default())
            .app_data(web::Data::new(state))
            .configure(app::register_urls)
    })
    .workers(1);

    // CHeck if TLS is enabled
    if args.tls {
        // Generate self-signed certificate | Unwrap here cause we cannot continue if it fails
        let cert = utils::utils::generate_self_signed_cert(&args.host).unwrap();

        // Create bind address
        let bind_address = format!("{}:{}", args.host, args.port);

        server.bind_rustls_0_23(bind_address, cert)?.run().await
    } else {
        server.bind((args.host, args.port))?.run().await
    }
}
