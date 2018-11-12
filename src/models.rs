extern crate bible_reference_rs;

#[derive(Debug)]
pub struct BookRef {
    pub id: i16,
    pub name: String,
    pub alt: String,
    pub locations: Vec<bible_reference_rs::VerseLocation>,
}
