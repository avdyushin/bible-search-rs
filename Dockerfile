FROM rustlang/rust:nightly
MAINTAINER <avdyushin.g@gmail.com>

RUN mkdir -p /usr/src/app
WORKDIR /usr/src/app

COPY . /usr/src/app

RUN rustc --version
RUN cargo install --path .

CMD ["bible-search-rs"]
