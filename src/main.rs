extern crate bible_reference_rs;
extern crate futures;
extern crate hyper;
extern crate postgres;
extern crate url;

mod models;
use futures::future;
use hyper::rt::Future;
use hyper::service::{NewService, Service};
use hyper::{Body, Error, Method, Request, Response, Server, StatusCode};
use models::*;
use postgres::{Connection, TlsMode};
use std::env;

const DEFAULT_URL: &'static str = "postgres://docker:docker@localhost:5432/bibles";

fn parse_query(query: &str) -> Option<String> {
    use std::collections::HashMap;

    let args = url::form_urlencoded::parse(&query.as_bytes())
        .into_owned()
        .collect::<HashMap<String, String>>();

    args.get("q").map(|v| v.to_string())
}

type BoxFut = Box<Future<Item = Response<Body>, Error = hyper::Error> + Send>;

fn echo(req: Request<Body>) -> BoxFut {
    let mut response = Response::new(Body::empty());

    match (req.method(), req.uri().path()) {
        (&Method::GET, "/") => match req.uri().query() {
            Some(query) => match parse_query(query) {
                Some(query) => *response.body_mut() = Body::from(format!("ask? {:?}", query)),
                None => *response.body_mut() = Body::from("Boo"),
            },
            None => *response.body_mut() = Body::from("Foo"),
        },
        (&Method::POST, "/echo") => {
            *response.body_mut() = req.into_body();
        }
        _ => {
            *response.status_mut() = StatusCode::NOT_FOUND;
        }
    };

    Box::new(future::ok(response))
}

struct SearchService;

impl NewService for SearchService {
    type ReqBody = Body;
    type ResBody = Body;
    type Error = Error;
    type Service = SearchService;
    type Future = Box<Future<Item = Self::Service, Error = Self::Error> + Send>;
    type InitError = Error;

    fn new_service(&self) -> Self::Future {
        Box::new(future::ok(SearchService))
    }
}

impl Service for SearchService {
    type ReqBody = Body;
    type ResBody = Body;
    type Error = Error;
    type Future = Box<Future<Item = Response<Self::ResBody>, Error = Self::Error> + Send>;

    fn call(&mut self, request: Request<Self::ReqBody>) -> Self::Future {
        println!("Get request: {:?}", request);
        echo(request)
    }
}

fn main() {
    let addr = "127.0.0.1:8080".parse().unwrap();

    let server = Server::bind(&addr)
        .serve(SearchService)
        .map_err(|e| eprintln!("server error: {}", e));

    println!("Listening {}", addr);
    hyper::rt::run(server);

    let url = env::var("DATABASE_URL").unwrap_or(String::from(DEFAULT_URL));
    println!("Connecting: {}", &url);

    let conn = Connection::connect(url, TlsMode::None).unwrap();
    let books = conn
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
}
