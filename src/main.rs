use actix_web::{App, HttpServer, middleware::Logger, web};
use clap::Parser;
use env_logger::Env;
use ifcfg::IfCfg;
use std::net::SocketAddr;
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

fn print_startup_messages(args: &Args) {
    let scheme = if args.tls { "https" } else { "http" };

    // Check if binding to all interfaces
    if args.host == "0.0.0.0" || args.host == "::" {
        println!(" * Running on all addresses ({})", args.host);
        match IfCfg::get() {
            Ok(ifaces) => {
                let mut found_specific_ip = false;
                for iface in ifaces {
                    // Skip loopback interfaces
                    if iface.name == "lo" || iface.name == "lo0" {
                        log::debug!("Skipping loopback interface {}", iface.name);
                        continue;
                    }

                    // Still print loopback for 0.0.0.0 case as it's reachable
                    for addr_info in &iface.addresses {
                        if let Some(SocketAddr::V4(v4_addr)) = addr_info.address {
                            if v4_addr.ip().is_loopback() {
                                println!(
                                    " * Running on {}://{}:{}",
                                    scheme,
                                    v4_addr.ip(),
                                    args.port
                                );
                                found_specific_ip = true;
                            }
                        }
                    }

                    // Skip common virtual/docker interface prefixes, but allow 'tun' as requested
                    let name_lower = iface.name.to_lowercase();
                    if name_lower.starts_with("docker")
                        || name_lower.starts_with("veth")
                        || name_lower.starts_with("vmnet")
                        || name_lower.starts_with("virbr")
                    // Add other interface name patterns to exclude if needed
                    {
                        log::debug!("Skipping interface {} due to filter", iface.name);
                        continue;
                    }

                    // Print IPv4 addresses for the interface
                    for addr_info in iface.addresses {
                        if let Some(SocketAddr::V4(v4_addr)) = addr_info.address {
                            // Avoid re-printing loopback if already handled or if it's not the primary address for 0.0.0.0
                            if !v4_addr.ip().is_loopback() {
                                // Only print non-loopback here
                                println!(
                                    " * Running on {}://{}:{}",
                                    scheme,
                                    v4_addr.ip(),
                                    args.port
                                );
                                found_specific_ip = true;
                            }
                        }
                        // Optionally, handle and print IPv6 addresses if args.host == "::"
                        // if args.host == "::" {
                        //     if let Some(SocketAddr::V6(v6_addr)) = addr_info.address {
                        //         println!(" * Running on {}://[{}]:{}", scheme, v6_addr.ip(), args.port);
                        //         found_specific_ip = true;
                        //     }
                        // }
                    }
                }
                if !found_specific_ip && (args.host == "0.0.0.0" || args.host == "::") {
                    // Fallback if no specific IPs were found but listening on all, maybe only loopback was available
                    // This case should be rare if loopback is explicitly printed above.
                    log::warn!(
                        "Could not find specific non-loopback IP addresses, but server is listening on all interfaces."
                    );
                }
            }
            Err(e) => {
                log::error!("[ERROR] Could not get network interfaces: {}", e);
                // Fallback to just printing the bind address as specified
                println!(
                    " * Running on {}://{}:{} (interface detection failed)",
                    scheme, args.host, args.port
                );
            }
        }
    } else {
        // Specific host provided
        println!(" * Running on {}://{}:{}", scheme, args.host, args.port);
    }
    println!("Press CTRL+C to quit\n");
}

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

    // Print startup messages before starting the server
    print_startup_messages(&Args::parse());

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
