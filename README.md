# Bible Microservice REST API in Rust

Bible text, references and search as REST API microservice packed into Docker container.

## Running server
With docker compose you can run server using `run.sh up` command.
This will up database container and API server container as one instance.

## Frontend client in JavaScript

Here [bible-search-js](https://github.com/avdyushin/bible-search-js) is one page simple JS client for this service.

## Usage
### Endpoints

#### Verse of The Day

Request:

`GET /daily`

Response:

```json
{
   "results" : [
      {
         "texts" : [
            [
               {
                  "book_id" : 51,
                  "text" : "Итак по плодам их узнаете их.",
                  "verse" : 20,
                  "chapter" : 7
               }
            ]
         ],
         "reference" : {
            "alt" : "Матф",
            "title" : "От Матфея"
         }
      },
   ]
}
```

#### References

Request:

`GET /refs?q=REF`,
where _REF_ is Bible reference, like `Быт 1:1`.

Response:

```json
{
   "results" : [
      {
         "reference" : {
            "alt" : "Быт",
            "title" : "Бытие"
         },
         "texts" : [
            [
               {
                  "text" : "В начале сотворил Бог небо и землю.",
                  "verse" : 1,
                  "book_id" : 1,
                  "chapter" : 1
               }
            ]
         ]
      }
   ]
}
```

#### Text search

Request:

`GET /search?q=TEXT`, where `TEXT` is text to look up.

Response:

```json
{
  "meta": {
    "page": 1,
    "text": "в начале было",
    "total": 1
  },
  "results": [
    [
      {
        "book_alt": "От Иоанна",
        "book_id": 54,
        "chapter": 1,
        "text": "В начале было Слово, и Слово было у Бога, и Слово было Бог.",
        "verse": 1
      }
    ]
  ]
}
```
