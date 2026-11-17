//! Typed streams: a stream is records of one named, versioned type, in an order the type gives a meaning to, and the
//! type's definition is shared by every writer and reader, so a record means the same whether a generator, a
//! customer's files or a script wrote it.

pub mod types;
