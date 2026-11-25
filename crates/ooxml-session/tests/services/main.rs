//! The session's services -- refs, facts, datasets and sources (design phase 3, section 5)
//! -- as one test program, on a made-up format (`TestFormat`): ooxml-edit's generic host
//! tests ported (design section 7, item 4). New service tests go in a module here.

mod datasets;
mod facts;
mod refs;
mod sources;
mod stores;
mod test_format;
mod toy_session;
