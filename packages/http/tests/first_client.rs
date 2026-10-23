// A separate test process starts without any application entry-point setup.
// This catches Reqwest 0.13's missing-provider panic in standalone consumers.
#[test]
fn clients_work_before_any_entry_point_initialization() {
    assert!(rustls::crypto::CryptoProvider::get_default().is_none());
    let _client = future_http::client();
    let provider = rustls::crypto::CryptoProvider::get_default().unwrap();
    let _other_client = future_http::client_builder().build().unwrap();
    assert!(std::ptr::eq(
        provider,
        rustls::crypto::CryptoProvider::get_default().unwrap()
    ));
    #[cfg(feature = "blocking")]
    let _blocking_client = future_http::blocking::client();
}
