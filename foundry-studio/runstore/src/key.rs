//! What a key may be spelled with, apart from what it names.
//!
//! Relative to the crate: `runstore` addresses objects by key,
//! [`layout`](crate::layout) decides which keys a run's objects get, and this
//! module decides which keys the store accepts at all. One alphabet, checked
//! here before any backend sees a key, so that a key either works on both
//! backends or is refused by both.
//!
//! The rule is narrower than what the backends would take, because the two
//! disagree outside it. `#` names the folder backend's staging files, which it
//! hides from its own listings while a bucket stores them as ordinary objects.
//! A leading or trailing `/` is silently rewritten, so `create("/a")` and
//! `create("a/")` both address `a` and a later listing hands back a key the
//! caller never wrote. `.` and `..` invite a reader to think a key is a
//! filesystem path. All of it is refused here, spelled out in the failure, and
//! never rewritten: a key that is not already canonical is not a key.

use thiserror::Error;

/// Reads a key: one or more segments separated by single `/`, with no leading
/// or trailing `/`.
pub fn validate_key(key: &str) -> Result<(), KeyError> {
    if key.is_empty() {
        return Err(KeyError::Empty);
    }
    if key.starts_with('/') {
        return Err(KeyError::LeadingSlash);
    }
    if key.ends_with('/') {
        return Err(KeyError::TrailingSlash);
    }
    key.split('/').try_for_each(validate_segment)
}

/// Reads a prefix: any leading substring of a key. It may be empty, may end in
/// `/`, and may stop part way through a segment, because a caller narrowing a
/// listing is naming the start of the keys it wants, not a key.
pub fn validate_prefix(prefix: &str) -> Result<(), KeyError> {
    if prefix.starts_with('/') {
        return Err(KeyError::LeadingSlash);
    }
    let mut segments = prefix.split('/').peekable();
    while let Some(segment) = segments.next() {
        match segments.peek() {
            // Anything before the last `/` is a whole segment.
            Some(_) => validate_segment(segment)?,
            // The last piece is as much of a segment as the caller typed.
            None => {
                if let Some(character) = disallowed_character(segment) {
                    return Err(KeyError::Character { character });
                }
            }
        }
    }
    Ok(())
}

/// Reads one segment: the piece between two slashes, and the shape a run id and
/// every other name in the layout has to take.
pub fn validate_segment(segment: &str) -> Result<(), KeyError> {
    if segment.is_empty() {
        return Err(KeyError::EmptySegment);
    }
    if let Some(character) = disallowed_character(segment) {
        return Err(KeyError::Character { character });
    }
    if segment == "." || segment == ".." {
        return Err(KeyError::DotSegment { segment: segment.to_string() });
    }
    Ok(())
}

/// The first character of `text` that is not in the alphabet, if any.
fn disallowed_character(text: &str) -> Option<char> {
    text.chars().find(|character| !is_allowed(*character))
}

/// The alphabet a segment is drawn from.
fn is_allowed(character: char) -> bool {
    character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-')
}

/// Why a key or prefix is not one this store will take.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum KeyError {
    /// The key has nothing in it.
    #[error("it names nothing")]
    Empty,
    /// The key or prefix starts with the separator.
    #[error("it begins with `/`")]
    LeadingSlash,
    /// The key ends with the separator, so its last segment is missing.
    #[error("it ends with `/`")]
    TrailingSlash,
    /// Two separators with nothing between them.
    #[error("it has an empty segment")]
    EmptySegment,
    /// A character outside the alphabet.
    #[error("`{character}` is not allowed; a segment is made of letters, digits, `.`, `_`, and `-`")]
    Character {
        /// The first character that is not in the alphabet.
        character: char,
    },
    /// A segment that reads as a filesystem's own directory names.
    #[error("`{segment}` does not name anything")]
    DotSegment {
        /// The segment, `.` or `..`.
        segment: String,
    },
}

#[cfg(test)]
mod tests {
    use super::{KeyError, validate_key, validate_prefix, validate_segment};

    #[test]
    fn a_key_is_segments_between_single_slashes() {
        for key in ["a", "runs/01J8Z5K3QW/spec.json", "runs/r-1_2/attempts/1/chunks/000000", "a.b-c_d/E9"] {
            assert_eq!(validate_key(key), Ok(()), "`{key}` should be a key");
        }
    }

    #[test]
    fn keys_the_backends_would_read_differently_are_refused() {
        // Each of these is taken by at least one backend and refused, rewritten,
        // or hidden by the other.
        assert_eq!(validate_key(""), Err(KeyError::Empty));
        assert_eq!(validate_key("/runs/a"), Err(KeyError::LeadingSlash));
        assert_eq!(validate_key("runs/a/"), Err(KeyError::TrailingSlash));
        assert_eq!(validate_key("runs//a"), Err(KeyError::EmptySegment));
        assert_eq!(validate_key("runs/a#1"), Err(KeyError::Character { character: '#' }));
        assert_eq!(validate_key("a#1"), Err(KeyError::Character { character: '#' }));
        assert_eq!(validate_key("runs/%2E%2E/x"), Err(KeyError::Character { character: '%' }));
        assert_eq!(validate_key("runs/../x"), Err(KeyError::DotSegment { segment: "..".to_string() }));
        assert_eq!(validate_key("runs/./x"), Err(KeyError::DotSegment { segment: ".".to_string() }));
        assert_eq!(validate_key("runs/a b"), Err(KeyError::Character { character: ' ' }));
        assert_eq!(validate_key("runs/año"), Err(KeyError::Character { character: 'ñ' }));
    }

    #[test]
    fn a_prefix_is_any_start_of_a_key() {
        // The empty prefix is the whole store; a prefix may stop inside a
        // segment, which is what makes a partial chunk sequence a usable filter.
        for prefix in ["", "runs", "runs/", "runs/r1/attempts/1/chunks/0000"] {
            assert_eq!(validate_prefix(prefix), Ok(()), "`{prefix}` should be a prefix");
        }
    }

    #[test]
    fn a_prefix_no_key_could_start_with_is_refused() {
        assert_eq!(validate_prefix("/"), Err(KeyError::LeadingSlash));
        assert_eq!(validate_prefix("/runs"), Err(KeyError::LeadingSlash));
        assert_eq!(validate_prefix("runs//"), Err(KeyError::EmptySegment));
        assert_eq!(validate_prefix("runs/../"), Err(KeyError::DotSegment { segment: "..".to_string() }));
        assert_eq!(validate_prefix("runs/a#"), Err(KeyError::Character { character: '#' }));
    }

    #[test]
    fn a_segment_is_one_name() {
        assert_eq!(validate_segment("01J8Z5K3QW"), Ok(()));
        assert_eq!(validate_segment(""), Err(KeyError::EmptySegment));
        assert_eq!(validate_segment("a/b"), Err(KeyError::Character { character: '/' }));
        assert_eq!(validate_segment(".."), Err(KeyError::DotSegment { segment: "..".to_string() }));
        assert_eq!(validate_segment("..."), Ok(()));
    }
}
