#[cfg(test)]
mod tests {
    use site_model::canonical::*;
    use serde_json::json;

    #[test]
    fn comments_vanish_at_every_depth() {
        let v = json!({
            "$comment": "top",
            "components": {
                "region": { "origin": [0, 0], "$comment": "inside a data map" },
                "$comment": "between components"
            },
            "pairs": [{ "$comment": "inside an array" , "key": "cell:0,0" }]
        });
        let s = to_canonical_string(&v);
        assert!(!s.contains("$comment"), "{s}");
        assert_eq!(
            s,
            r#"{"components":{"region":{"origin":[0,0]}},"pairs":[{"key":"cell:0,0"}]}"#
        );
    }

    #[test]
    fn keys_sort_and_whitespace_disappears() {
        let v = json!({ "b": 1, "a": { "d": 2, "c": 3 } });
        assert_eq!(to_canonical_string(&v), r#"{"a":{"c":3,"d":2},"b":1}"#);
    }

    #[test]
    fn whole_valued_floats_print_as_integers() {
        let v = json!({ "position": [5750.0, 6650.0], "half": 0.5, "zero": -0.0 });
        assert_eq!(
            to_canonical_string(&v),
            r#"{"half":0.5,"position":[5750,6650],"zero":0}"#
        );
    }

    #[test]
    fn control_characters_escape() {
        let v = json!({ "name": "a\tb\u{1}c" });
        assert_eq!(to_canonical_string(&v), r#"{"name":"a\tb\u0001c"}"#);
    }
}
