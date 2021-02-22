use actix_web::{App, HttpServer};

mod generated_files {
    include!(concat!(env!("OUT_DIR"), "/generated.rs"));
}

#[actix_web::main]
async fn main() {
    HttpServer::new(move || {
        let generated = generated_files::generate();
        App::new().service(
            actix_web_static_files::ResourceFiles::new("/", generated).resolve_not_found_to_root(),
        )
    })
    .bind("127.0.0.1:8980")
    .unwrap()
    .bind("192.168.1.15:8980")
    .unwrap()
    .run()
    .await
    .unwrap();
}
