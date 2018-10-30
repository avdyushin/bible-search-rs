extern crate bible_reference_rs;
extern crate futures;
extern crate hyper;
extern crate postgres;
extern crate url;

mod models;
use futures::future::{Future, FutureResult};
use hyper::body::Payload;
use hyper::service::{NewService, Service};
use hyper::{Body, Error, Method, Request, Response, Server, StatusCode};
use models::*;
use postgres::{Connection, TlsMode};
use std::env;

const DEFAULT_URL: &'static str = "postgres://docker:docker@localhost:5432/bibles";

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

    // Verse Of the Day
    fn vod_response(&self) -> Body {
        Body::from("Gen 1:1")
    }
}

fn parse_query(query: Option<&str>) -> FutureResult<String, Error> {
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
        // TODO: Throw error
        None => futures::future::ok(String::from("error")),
    }
}

fn parse_body(body: Body) -> FutureResult<String, Error> {
    if body.content_length().unwrap_or(0) == 0 {
        // TODO: Throw error
        futures::future::ok(String::from("empty"))
    } else {
        // TODO: Extract body string value
        futures::future::ok(String::from("que"))
    }
}

fn search_results(query: String) -> FutureResult<Body, Error> {
    // TODO: Parse query into searches
    futures::future::ok(Body::from(format!("Results for {:?}", query)))
}

// TODO: Find results function

fn search_response(body: Result<Body, Error>) -> FutureResult<Response<Body>, Error> {
    match body {
        Ok(body) => futures::future::ok(Response::new(body)),
        // TODO: Show empty results
        Err(err) => futures::future::ok(Response::new(Body::from("OK"))),
    }
}

impl NewService for SearchService {
    type ReqBody = Body;
    type ResBody = Body;
    type Error = Error;
    type Service = SearchService;
    type Future = Box<Future<Item = Self::Service, Error = Self::Error> + Send>;
    type InitError = Error;

    fn new_service(&self) -> Self::Future {
        Box::new(futures::future::ok(SearchService))
    }
}

impl Service for SearchService {
    type ReqBody = Body;
    type ResBody = Body;
    type Error = Error;
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
            (&Method::POST, "/") => Box::new(
                parse_body(request.into_body())
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
