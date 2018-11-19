extern crate bible_reference_rs;
extern crate chrono;
extern crate futures;
extern crate hyper;
extern crate postgres;
extern crate serde;
extern crate url;
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
                None
            } else {
                let row = rows.iter().next().unwrap();
                Some(BookRef {
                    id: row.get(0),
                    name: row.get(1),
                    alt: row.get(2),
                    locations: r.locations.clone(),
                })
            }
        }).collect();

    valid
        .iter()
        .map(|reference| {
            let book_id = reference.id;
            let book_title = &reference.name;
            let book_alt = &reference.alt;
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
            json!({ "reference": { "title": book_title, "alt": book_alt }, "texts": texts })
        }).collect::<Vec<_>>()
}

fn fetch_daily_verses(db: &Connection) -> Vec<String> {
    use chrono::{Datelike, Utc};

    let now = Utc::now();
    let month = now.month() as i16;
    let day = now.day() as i16;

    db.query(
        "SELECT verses
         FROM rst_bible_daily
         WHERE month = $1 AND day = $2",
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

#[derive(Debug)]
struct SearchPaginate {
    text: String,
    page: i16,
}

fn parse_query_paginate(query: Option<&str>) -> FutureResult<SearchPaginate, ServiceError> {
    use std::collections::HashMap;

    let query = &query.unwrap_or("");
    let args = url::form_urlencoded::parse(&query.as_bytes())
        .into_owned()
        .collect::<HashMap<String, String>>();

    let q = args
        .get("q")
        .map(|v| v.to_string())
        .filter(|s| !s.is_empty());

    let p = args
        .get("p")
        .map(|v| v.parse::<i16>().unwrap_or(1))
        .unwrap_or(1);

    match (q, p) {
        (Some(q), p) => futures::future::ok(SearchPaginate { text: q, page: p }),
        _ => futures::future::err(ServiceError::NoInput),
    }
}

// Verse Of the Day
fn vod_response_body(db: &Connection) -> Body {
    let results = fetch_daily_verses(&db)
        .into_iter()
        .flat_map(|daily| {
            let refs = parse(daily.as_str());
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
    let refs = parse(query.as_str());
    futures::future::ok(Body::from(
        json!({ "results": fetch_results(&db, refs) }).to_string(),
    ))
}

fn fetch_search_results(text: String, page: i16, db: &Connection) -> (Vec<Value>, i64) {
    let page = if page <= 0 { 1 } else { page };

    let count_rows = db
        .query(
            "SELECT COUNT(book_id)
             FROM rst_bible
             WHERE text ~* $1",
            &[&text],
        ).unwrap();

    let mut total: i64 = 0;
    if count_rows.is_empty() {
        return (vec![json!([])], total);
    } else {
        total = count_rows.get(0).get("count");
    }

    let offset = ((page - 1) * 10) as i64;
    let rows = db
        .query(
            "SELECT row_to_json(t)
             FROM (
                SELECT v.book_id, v.text, v.chapter, v.verse, b.book as book_name, b.alt as book_alt from rst_bible v
                LEFT OUTER JOIN rst_bible_books b on (v.book_id = b.id)
                WHERE text ~* $1
             ) t
             LIMIT 10
             OFFSET $2",
            &[&text, &offset],
        ).unwrap();

    let results = rows.into_iter().map(|r| r.get(0)).collect::<Vec<Value>>();

    (vec![json!(results)], (total as f64 / 10_f64).ceil() as i64)
}

fn search_text(query: SearchPaginate, db: &Connection) -> FutureResult<Body, ServiceError> {
    let text = &query.text;
    let results = fetch_search_results(text.to_string(), query.page, db);

    futures::future::ok(Body::from(
        json!({
            "meta": { "text": text, "page": query.page, "total": results.1 },
            "results": results.0
        }).to_string(),
    ))
}

fn success_response(body: Body) -> FutureResult<Response<Body>, ServiceError> {
    futures::future::ok(
        Response::builder()
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
            .header(header::ACCESS_CONTROL_ALLOW_METHODS, "GET")
            .header(header::ACCESS_CONTROL_ALLOW_HEADERS, "Content-Type")
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
            (&Method::GET, "/refs") => Box::new(
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
            (&Method::GET, "/search") => Box::new(
                parse_query_paginate(request.uri().query())
                    .and_then(move |query| search_text(query, &db_connection))
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
        let refs = parse("Быт 1");
        let verses = fetch_results(&db, refs);
        assert_eq!(verses.len(), 1);
    }
}
