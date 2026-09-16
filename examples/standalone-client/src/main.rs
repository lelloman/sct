fn main() {
    let _client = sct_client::Client::new("http://localhost:8080".parse().unwrap()).unwrap();
    println!("ScT public API: {}", sct_protocol::API_PREFIX);
}
