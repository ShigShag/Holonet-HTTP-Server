use rcgen::generate_simple_self_signed;
use rustls::ServerConfig;
use std::io::BufReader;

use super::errors::UtilsError;

pub fn generate_self_signed_cert(ip: &String) -> Result<ServerConfig, UtilsError> {
    // Generate a self-signed certificate
    let cert = generate_simple_self_signed(vec![ip.into()])?;

    let cert_der = cert.cert.pem();
    let private_key_der = cert.key_pair.serialize_pem();

    let certs = &mut BufReader::new(cert_der.as_bytes());
    let private_key = &mut BufReader::new(private_key_der.as_bytes());

    // Load the certificate and private key
    let tls_certs = rustls_pemfile::certs(certs).collect::<Result<Vec<_>, _>>()?;

    let tls_key = match rustls_pemfile::pkcs8_private_keys(private_key).next() {
        Some(Ok(key)) => key,
        Some(Err(err)) => return Err(UtilsError::from(err)),
        _ => {
            return Err(UtilsError::UnknownType("No private key found".to_string()));
        }
    };

    // Configure rustls with the certificate and private key
    Ok(ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(tls_certs, rustls::pki_types::PrivateKeyDer::Pkcs8(tls_key))?)
}
