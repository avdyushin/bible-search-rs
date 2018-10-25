extern crate postgres;
mod models;
use postgres::{Connection, TlsMode};
use models::*;
use std::env;

const DEFAULT_URL: &'static str = "postgres://docker:docker@localhost:5432/bibles";

fn main() {

    let url = env::var("DATABASE_URL").unwrap_or(String::from(DEFAULT_URL));
    println!("Connecting: {}", &url);

    let conn = Connection::connect(url, TlsMode::None).unwrap();
    let books = conn.query("SELECT id, book, alt, abbr FROM rst_bible_books", &[]).unwrap();
    for row in &books {
        let book = Book {
            id: row.get(0),
            book: row.get(1),
            alt: row.get(2),
            abbr: row.get(3),
        };
        println!("{:?}", book);
    }
}
