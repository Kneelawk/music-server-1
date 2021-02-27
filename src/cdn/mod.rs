use actix_web::App;

pub mod index;

pub fn apply_services<T, B>(app: App<T, B>) -> App<T, B> {
    index::apply_services(app)
}
