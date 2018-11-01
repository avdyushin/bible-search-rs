extern crate bible_reference_rs;
extern crate chrono;
extern crate futures;
extern crate hyper;
extern crate postgres;
extern crate url;

#[macro_use]
extern crate serde_derive;
extern crate serde;
#[macro_use]
extern crate serde_json;

mod models;
use bible_reference_rs::*;
use chrono::{Datelike, Utc};
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

fn fetch_results(connection: Connection, refs: Vec<BibleReference>) -> String {
    let books = connection
        .query("SELECT id, book, alt, abbr FROM rst_bible_books", &[])
        .unwrap();

    for row in &books {
        let book = Book {
            id: row.get(0),
            book: row.get(1),
            alt: row.get(2),
            abbr: row.get(3),
        };
        println!("{:?}", book);
    }

    String::from("OK")
}

fn fetch_daily_verses(db_connection: &Connection) -> Vec<String> {
    let now = Utc::now();
    let month = now.month() as i16;
    let day = now.day() as i16;

    db_connection
        .query(
            "SELECT verses FROM rst_bible_daily WHERE month = $1 AND day = $2",
            &[&month, &day],
        ).unwrap()
        .iter()
        .map(|row| row.get(0))
        .collect()
}

// Verse Of the Day
fn vod_response_body(db_connection: &Connection) -> Body {
    // TODO: Fetch texts
    let daily = fetch_daily_verses(&db_connection);
    for verses in &daily {
        println!("Daily: {}", verses)
    }
    let results = json!({
        "results": "Verse of The Day: Gen 1:1"
    });
    Body::from(results.to_string())
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

fn search_results(
    query: Result<String, ServiceError>,
    db_connection: &Connection,
) -> FutureResult<Body, ServiceError> {
    match query {
        Ok(query) => {
            let refs = bible_reference_rs::parse(query.as_str());
            if refs.is_empty() {
                let empty = json!({});
                futures::future::ok(Body::from(empty.to_string()))
            } else {
                let results = json!({
                    "query": query,
                    "results": format!("{:?}", refs)
                });
                futures::future::ok(Body::from(results.to_string()))
            }
        }
        _ => futures::future::ok(vod_response_body(&db_connection)),
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
        println!("Got request: {:?}", request);

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
        .map_err(|e| eprintln!("server error: {}", e));

    println!("Listening {}", addr);
    hyper::rt::run(server);
}
