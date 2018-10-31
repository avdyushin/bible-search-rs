extern crate bible_reference_rs;
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
use futures::future::{Future, FutureResult};
use hyper::service::{NewService, Service};
use hyper::{header, Body, Method, Request, Response, Server, StatusCode};
use models::*;
use postgres::{Connection, TlsMode};
use std::env;
use std::fmt;

const DEFAULT_URL: &'static str = "postgres://docker:docker@localhost:5432/bibles";

#[derive(Debug)]
enum ServiceError {
    NoInput,
}
impl std::error::Error for ServiceError {}
impl fmt::Display for ServiceError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Error")
    }
}

struct SearchService;

impl SearchService {
    fn connect_db(&self) -> Option<Connection> {
        let url = env::var("DATABASE_URL").unwrap_or(String::from(DEFAULT_URL));

        println!("Connecting: {}", &url);
        match Connection::connect(url, TlsMode::None) {
            Ok(connection) => {
                println!("Success");
                Some(connection)
            }
            Err(error) => {
                println!("Error: {}", error);
                None
            }
        }
    }

    fn fetch_results(&self, db: Connection, query: Option<String>) -> String {
        let books = db
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
}

// Verse Of the Day
fn vod_response_body() -> Body {
    Body::from("Gen 1:1")
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

fn search_results(query: String) -> FutureResult<Body, ServiceError> {
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

// TODO: Find results function

fn search_response(body: Result<Body, ServiceError>) -> FutureResult<Response<Body>, ServiceError> {
    let body = match body {
        Ok(body) => body,
        Err(_) => vod_response_body(),
    };
    futures::future::ok(
        Response::builder()
            .header(header::CONTENT_TYPE, "application/json")
            .body(body)
            .unwrap(),
    )
}

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

        let db = match self.connect_db() {
            Some(db) => Some(db),
            None => {
                println!("Error getting DB connection");
                None
            }
        };

        match (request.method(), request.uri().path()) {
            (&Method::GET, "/") => Box::new(
                parse_query(request.uri().query())
                    .and_then(search_results)
                    .then(search_response),
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
