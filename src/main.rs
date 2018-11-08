extern crate bible_reference_rs;
extern crate chrono;
extern crate futures;
extern crate hyper;
extern crate postgres;
extern crate url;

//#[macro_use]
//extern crate serde_derive;
extern crate serde;
#[macro_use]
extern crate serde_json;

mod models;
use bible_reference_rs::*;
use futures::future::{Future, FutureResult};
use hyper::service::{NewService, Service};
use hyper::{header, Body, Method, Request, Response, Server, StatusCode};
use models::*;
use postgres::{Connection, TlsMode};
use std::env;
use std::fmt;

const DEFAULT_URL: &'static str = "postgres://docker:docker@localhost:5432/bible";

#[derive(Debug)]
enum ServiceError {
    NoInput,
    NoDatabaseConnection(String),
}
impl std::error::Error for ServiceError {}
impl fmt::Display for ServiceError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            ServiceError::NoInput => write!(f, "No input provided"),
            ServiceError::NoDatabaseConnection(details) => write!(f, "DB: {}", details),
        }
    }
}

fn connect_db() -> Result<Connection, ServiceError> {
    let url = env::var("DATABASE_URL").unwrap_or(String::from(DEFAULT_URL));

    println!("Connecting: {}", &url);
    match Connection::connect(url, TlsMode::None) {
        Ok(connection) => Ok(connection),
        Err(error) => {
            println!("Connection: {}", error);
            Err(ServiceError::NoDatabaseConnection(format!("{}", error)))
        }
    }
}

// TODO: Find results function

#[derive(Debug, Clone, Hash, Eq, PartialEq)]
pub struct Verse {
    pub book_id: i16,
    pub chapter: i16,
    pub verse: i16,
    pub text: String,
}

fn verses_in_chapter(db: &Connection, id: i16, chapter: i16) -> Vec<Verse> {
    db.query(
        "SELECT verse, text FROM rst_bible WHERE book_id = $1 AND chapter = $2",
        &[&id, &chapter],
    ).unwrap()
    .iter()
    .map(|row| Verse {
        book_id: id,
        chapter: chapter,
        verse: row.get(0),
        text: row.get(1),
    }).collect()
}

fn fetch_results(db: &Connection, refs: Vec<BibleReference>) -> String {
    println!("Fetch for {:?}", refs);

    let valid: Vec<BookRef> = refs
        .iter()
        .flat_map(|r| {
            let statement = db
                .prepare(
                    "SELECT id, book as title, alt, abbr
                     FROM rst_bible_books
                     WHERE book ~* $1 OR alt ~* $1 OR abbr ~* $1
                     LIMIT 1",
                ).unwrap();

            let rows = statement.query(&[&r.book]).unwrap();
            if rows.is_empty() {
                Err("ok")
            } else {
                let row = rows.iter().next().unwrap();
                Ok(BookRef {
                    id: row.get(0),
                    locations: r.locations.clone(),
                })
            }
        }).collect();

    let flatten = valid
        .into_iter()
        .map(|reference| {
            let book_id = reference.id;
            reference
                .locations
                .iter()
                .flat_map(move |l| match (&l.chapters, &l.verses) {
                    (chapters, None) if chapters.len() == 1 => {
                        let ch = chapters[0] as i16;
                        Some(verses_in_chapter(&db, book_id, ch))
                    }
                    (chapters, _verses) if chapters.len() == 1 => {
                        println!("Verses set in chapter");
                        None
                    }
                    _ => None,
                }).collect::<Vec<_>>()
        }).flatten()
        .collect::<Vec<_>>();

    println!("r {:?}", flatten);

    let results = json!({ "results": format!("{:?}", refs) });

    results.to_string()
}

fn fetch_daily_verses(db: &Connection) -> Vec<String> {
    use chrono::{Datelike, Utc};

    let now = Utc::now();
    let month = now.month() as i16;
    let day = now.day() as i16;

    db.query(
        "SELECT verses FROM rst_bible_daily WHERE month = $1 AND day = $2",
        &[&month, &day],
    ).unwrap()
    .iter()
    .map(|row| row.get(0))
    .collect()
}

fn parse_query(query: Option<&str>) -> FutureResult<String, ServiceError> {
    use std::collections::HashMap;

    let query = &query.unwrap_or("");
    let args = url::form_urlencoded::parse(&query.as_bytes())
        .into_owned()
        .collect::<HashMap<String, String>>();

    match args
        .get("q")
        .map(|v| v.to_string())
        .filter(|s| !s.is_empty())
    {
        Some(value) => futures::future::ok(value),
        None => futures::future::err(ServiceError::NoInput),
    }
}

// Verse Of the Day
fn vod_response_body(db: &Connection) -> Body {
    // TODO: Fetch texts
    let daily = fetch_daily_verses(&db);
    for verses in &daily {
        println!("Daily: {}", verses)
    }
    let results = json!({
        "results": "Verse of The Day: Gen 1:1"
    });
    Body::from(results.to_string())
}

fn search_results(
    query: Result<String, ServiceError>,
    db: &Connection,
) -> FutureResult<Body, ServiceError> {
    match query {
        Ok(query) => {
            let refs = bible_reference_rs::parse(query.as_str());
            if refs.is_empty() {
                let empty = json!({});
                futures::future::ok(Body::from(empty.to_string()))
            } else {
                futures::future::ok(Body::from(fetch_results(&db, refs)))
            }
        }
        _ => futures::future::ok(vod_response_body(&db)),
    }
}

fn search_response(body: Body) -> FutureResult<Response<Body>, ServiceError> {
    futures::future::ok(
        Response::builder()
            .header(header::CONTENT_TYPE, "application/json")
            .body(body)
            .unwrap(),
    )
}

struct SearchService;

impl NewService for SearchService {
    type ReqBody = Body;
    type ResBody = Body;
    type Error = ServiceError;
    type Service = SearchService;
    type Future = Box<Future<Item = Self::Service, Error = Self::Error> + Send>;
    type InitError = ServiceError;

    fn new_service(&self) -> Self::Future {
        Box::new(futures::future::ok(SearchService))
    }
}

impl Service for SearchService {
    type ReqBody = Body;
    type ResBody = Body;
    type Error = ServiceError;
    type Future = Box<Future<Item = Response<Self::ResBody>, Error = Self::Error> + Send>;

    fn call(&mut self, request: Request<Self::ReqBody>) -> Self::Future {
        let db_connection = match connect_db() {
            Ok(db) => db,
            Err(_) => {
                return Box::new(futures::future::ok(
                    Response::builder()
                        .status(StatusCode::INTERNAL_SERVER_ERROR)
                        .body(Body::empty())
                        .unwrap(),
                ))
            }
        };

        match (request.method(), request.uri().path()) {
            (&Method::GET, "/") => Box::new(
                parse_query(request.uri().query())
                    .then(move |query| search_results(query, &db_connection))
                    .and_then(search_response),
            ),
            _ => Box::new(futures::future::ok(
                Response::builder()
                    .status(StatusCode::NOT_FOUND)
                    .body(Body::empty())
                    .unwrap(),
            )),
        }
    }
}

fn main() {
    let addr = "127.0.0.1:8080".parse().unwrap();
    let server = Server::bind(&addr)
        .serve(SearchService)
        .map_err(|e| eprintln!("Server error: {}", e));

    println!("Listening {}", addr);
    hyper::rt::run(server);
}

#[cfg(test)]

mod tests {
    use super::*;

    #[test]
    fn test_fetch_chapter() {
        let db = connect_db().unwrap();
        let refs = bible_reference_rs::parse("Быт 1");
        fetch_results(&db, refs);
    }
}
