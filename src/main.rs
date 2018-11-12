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
use serde_json::Value;
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

fn verses_by_chapters(db: &Connection, id: i16, chapters: Vec<i16>) -> Vec<Value> {
    db.query(
        "SELECT row_to_json(rst_bible)
         FROM rst_bible
         WHERE book_id = $1 AND chapter = ANY($2)",
        &[&id, &chapters],
    ).unwrap()
    .iter()
    .map(|row| row.get(0))
    .collect()
}

fn verses_in_chapter_by_verses(
    db: &Connection,
    id: i16,
    chapter: i16,
    verses: Vec<i16>,
) -> Vec<Value> {
    db.query(
        "SELECT row_to_json(rst_bible)
         FROM rst_bible
         WHERE book_id = $1 AND chapter = $2 AND verse = ANY($3)",
        &[&id, &chapter, &verses],
    ).unwrap()
    .iter()
    .map(|row| row.get(0))
    .collect()
}

fn fetch_results(db: &Connection, refs: Vec<BibleReference>) -> Vec<Value> {
    println!("Fetch for {:?}", refs);

    if refs.is_empty() {
        return vec![];
    }

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

    let results = valid
        .iter()
        .map(|reference| {
            let book_id = reference.id;
            let texts = reference
                .locations
                .iter()
                .flat_map(
                    move |location| match (&location.chapters, &location.verses) {
                        // Fetch verses by chapters
                        (chapters, None) => {
                            let ch = chapters.into_iter().map(|v| *v as i16).collect();
                            Some(verses_by_chapters(&db, book_id, ch))
                        }
                        // Fetch verses by chapter and verses
                        (chapters, Some(verses)) if chapters.len() == 1 => {
                            let ch = chapters[0] as i16;
                            let vs = verses.into_iter().map(|v| *v as i16).collect();
                            Some(verses_in_chapter_by_verses(&db, book_id, ch, vs))
                        }
                        _ => None,
                    },
                ).collect::<Vec<_>>();
            json!({ "ref_for": book_id, "texts": texts })
        }).collect::<Vec<_>>();

    results
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
    let results = fetch_daily_verses(&db)
        .into_iter()
        .flat_map(|daily| {
            let refs = bible_reference_rs::parse(daily.as_str());
            let results = fetch_results(&db, refs);
            if results.is_empty() {
                None
            } else {
                Some(results)
            }
        }).flatten()
        .collect::<Vec<_>>();

    Body::from(json!({ "results": results }).to_string())
}

fn search_results(query: String, db: &Connection) -> FutureResult<Body, ServiceError> {
    let refs = bible_reference_rs::parse(query.as_str());
    futures::future::ok(Body::from(
        json!({ "results": fetch_results(&db, refs) }).to_string(),
    ))
}

fn success_response(body: Body) -> FutureResult<Response<Body>, ServiceError> {
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
                    .and_then(move |query| search_results(query, &db_connection))
                    .and_then(success_response)
                    .or_else(|_| {
                        futures::future::ok(
                            Response::builder()
                                .status(StatusCode::BAD_REQUEST)
                                .body(Body::empty())
                                .unwrap(),
                        )
                    }),
            ),
            (&Method::GET, "/daily") => {
                Box::new(success_response(vod_response_body(&db_connection)))
            }
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
