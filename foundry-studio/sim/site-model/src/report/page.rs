//! The page itself: the one file that gets shared, and the escaping that keeps
//! a document's own text from becoming part of it.
//!
//! Everything bounded is written here as markup — the summary, the documents,
//! the component census, the per-generator expansion counts, the warnings and
//! errors — so somebody who opens the file reads the findings before a line of
//! script has run. The unbounded half, the entity tree, is left to the script
//! and the view model beside it: a world of 21,000 entities cannot be laid out
//! ahead of being asked for.
//!
//! Two rules hold everywhere below. Anything that came out of a document —
//! names, paths, component names, the text of a warning — goes through
//! [`escape`] before it is written into markup, so a name containing `<script>`
//! arrives as five visible characters and not as an element. And the view model
//! is embedded as JSON inside a script element the browser never executes,
//! with `<`, `>` and `&` written as their JSON escapes, so no string in any
//! document can end that element early.

use super::model::{DocumentView, Page};
use std::fmt::Write as _;
use std::path::Path;

const STYLE: &str = include_str!("report.css");
const SCRIPT: &str = include_str!("report.js");

/// Write the whole file.
pub fn render(page: &Page) -> String {
    let mut out = String::with_capacity(64 * 1024);
    out.push_str("<!doctype html>\n<html lang=\"en\">\n<head>\n");
    out.push_str("<meta charset=\"utf-8\">\n");
    out.push_str("<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n");
    let _ = writeln!(out, "<title>{}</title>", escape(&title(page)));
    let _ = writeln!(out, "<style>\n{STYLE}</style>");
    out.push_str("</head>\n<body>\n");

    header(&mut out, page);
    out.push_str("<main>\n");
    if page.errors.is_empty() {
        summary(&mut out, page);
        documents(&mut out, page);
        findings(&mut out, page);
        census(&mut out, page);
        expansion(&mut out, page);
        tree(&mut out);
        suppressions(&mut out, page);
    } else {
        findings(&mut out, page);
    }
    out.push_str("</main>\n");

    let _ = writeln!(
        out,
        "<script type=\"application/json\" id=\"world-data\">{}</script>",
        embed(&super::model::to_json(page))
    );
    let _ = writeln!(out, "<script>\n{SCRIPT}</script>");
    out.push_str("</body>\n</html>\n");
    out
}

/// What the file is called in a browser tab: the scenario as its own document
/// metadata names it, or the file name when that is all there is — a whole
/// path across a tab tells the reader nothing.
fn title(page: &Page) -> String {
    let subject = page.scenario.name.clone().unwrap_or_else(|| {
        Path::new(&page.scenario.input)
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| page.scenario.input.clone())
    });
    if page.errors.is_empty() {
        format!("{subject} — world structure")
    } else {
        format!("{subject} — refused to load")
    }
}

fn header(out: &mut String, page: &Page) {
    out.push_str("<header>\n<div class=\"masthead\">\n<div>\n");
    let _ = writeln!(out, "<h1>{}</h1>", escape(&title(page)));
    let _ = writeln!(
        out,
        "<p class=\"path\">{}</p>",
        escape(&page.scenario.input)
    );
    out.push_str("</div>\n");
    if page.errors.is_empty() {
        out.push_str("<div class=\"find\">\n");
        out.push_str(
            "<input id=\"find\" type=\"search\" placeholder=\"find an entity: name, UUID, or key \
             segment\" autocomplete=\"off\" spellcheck=\"false\">\n",
        );
        out.push_str("<div id=\"find-results\"></div>\n</div>\n");
    }
    out.push_str("</div>\n");
    out.push_str(if page.errors.is_empty() {
        "<p class=\"lede\">What this world is made of — the entities, the components they \
         carry, what derived what, and what points at what. Not a map: nothing here is drawn \
         to scale or to position.</p>\n"
    } else {
        "<p class=\"lede\">This scenario does not load, so there is no world to describe. \
         What follows is the loader's refusal, word for word, and whatever it names as the \
         cause.</p>\n"
    });
    out.push_str("</header>\n");
}

fn summary(out: &mut String, page: &Page) {
    let s = &page.summary;
    out.push_str("<section id=\"summary\">\n<h2>Summary</h2>\n<div class=\"tiles\">\n");
    tile(
        out,
        "entities",
        &number(s.entities),
        &format!(
            "{} declared, {} derived",
            number(s.declared),
            number(s.derived)
        ),
    );
    tile(
        out,
        "edges",
        &number(s.edges),
        &format!(
            "{} directed flow arcs, {} bidirectional stitch links",
            number(s.flow_arcs),
            number(s.stitch_links)
        ),
    );
    tile(
        out,
        "generators",
        &number(s.generators),
        "entities whose components imply further entities",
    );
    tile(
        out,
        "components",
        &number(s.components),
        "distinct component names in use",
    );
    tile(
        out,
        "suppressed",
        &number(s.suppressed),
        "targets deleted by a record or an exclusion",
    );
    tile(
        out,
        "documents",
        &number(s.layers + 1),
        &format!("{} layers plus the scenario", number(s.layers)),
    );
    tile(
        out,
        "warnings",
        &number(s.warnings),
        "loadable, but maybe not meant",
    );
    tile(out, "errors", &number(s.errors), "refusals to load");
    out.push_str("</div>\n</section>\n");
}

fn tile(out: &mut String, label: &str, value: &str, note: &str) {
    let _ = writeln!(
        out,
        "<div class=\"tile\"><span class=\"tile-value\">{}</span>\
         <span class=\"tile-label\">{}</span>\
         <span class=\"tile-note\">{}</span></div>",
        escape(value),
        escape(label),
        escape(note)
    );
}

fn documents(out: &mut String, page: &Page) {
    out.push_str("<section id=\"documents\">\n<h2>Documents</h2>\n");
    out.push_str(
        "<p class=\"note\">The scenario and the layers it composes, in the order the loader \
         reads them: later layers replace earlier ones per component, and the scenario has the \
         last word.</p>\n",
    );
    out.push_str(
        "<table>\n<thead><tr><th>role</th><th>document</th><th>records</th>\
                  <th>required components</th></tr></thead>\n<tbody>\n",
    );
    for document in &page.documents {
        document_row(out, document);
    }
    out.push_str("</tbody>\n</table>\n");
    if let Some(settings) = &page.scenario.settings {
        let text = serde_json::to_string_pretty(settings).unwrap_or_else(|_| settings.to_string());
        let _ = writeln!(
            out,
            "<details class=\"payload\"><summary>scenario settings</summary><pre>{}</pre>\
             </details>",
            escape(&text)
        );
    }
    out.push_str("</section>\n");
}

fn document_row(out: &mut String, document: &DocumentView) {
    let records = if document.records.is_empty() {
        "none".to_string()
    } else {
        document
            .records
            .iter()
            .map(|(kind, count)| format!("{count} {kind}{}", if *count == 1 { "" } else { "s" }))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let required = if document.required.is_empty() {
        "none".to_string()
    } else {
        document.required.join(", ")
    };
    let kind = document
        .kind
        .as_deref()
        .map(|k| format!(" · {k}"))
        .unwrap_or_default();
    let _ = writeln!(
        out,
        "<tr><td>{}</td><td><div class=\"doc-name\">{}{}</div>\
         <div class=\"path\">{}</div><div class=\"uuid\">{}</div></td>\
         <td>{}</td><td>{}</td></tr>",
        escape(document.role),
        escape(document.name.as_deref().unwrap_or("unnamed")),
        escape(&kind),
        escape(&document.path),
        escape(&document.id),
        escape(&records),
        escape(&required)
    );
}

/// The warnings and the errors, word for word as the loader states them.
fn findings(out: &mut String, page: &Page) {
    out.push_str("<section id=\"findings\">\n");
    let _ = writeln!(
        out,
        "<h2>Errors <span class=\"count\">{}</span></h2>",
        number(page.summary.errors)
    );
    if page.errors.is_empty() {
        out.push_str("<p class=\"note\">The scenario loaded.</p>\n");
    } else {
        out.push_str("<ul class=\"findings errors\">\n");
        for error in &page.errors {
            let _ = writeln!(out, "<li>{}</li>", escape(error));
        }
        out.push_str("</ul>\n");
    }
    let _ = writeln!(
        out,
        "<h2>Warnings <span class=\"count\">{}</span></h2>",
        number(page.summary.warnings)
    );
    if page.warnings.is_empty() {
        out.push_str("<p class=\"note\">Nothing the loader thought unintended.</p>\n");
    } else {
        out.push_str("<ul class=\"findings warnings\">\n");
        for warning in &page.warnings {
            let _ = writeln!(out, "<li>{}</li>", escape(warning));
        }
        out.push_str("</ul>\n");
    }
    out.push_str("</section>\n");
}

fn census(out: &mut String, page: &Page) {
    out.push_str("<section id=\"census\">\n<h2>Components</h2>\n");
    out.push_str(
        "<p class=\"note\">The set of components an entity carries is its type; there is no \
         type field. This is how many entities carry each one.</p>\n",
    );
    out.push_str(
        "<table class=\"census\">\n<thead><tr><th>component</th><th class=\"num\">\
                  entities</th></tr></thead>\n<tbody>\n",
    );
    for (component, count) in &page.census {
        let _ = writeln!(
            out,
            "<tr><td><code>{}</code></td><td class=\"num\">{}</td></tr>",
            escape(component),
            escape(&number(*count))
        );
    }
    out.push_str("</tbody>\n</table>\n</section>\n");
}

fn expansion(out: &mut String, page: &Page) {
    out.push_str("<section id=\"expansion\">\n<h2>Expansion by generator</h2>\n");
    out.push_str(
        "<p class=\"note\">Every derived entity hangs under the declared entity whose \
         expansion produced it, addressed by the chain of key segments beneath it. Follow a \
         row to that generator's subtree.</p>\n",
    );
    out.push_str(
        "<table class=\"expansion\">\n<thead><tr><th class=\"num\">derived</th><th>generator\
         </th><th>components</th></tr></thead>\n<tbody>\n",
    );
    for generator in &page.generators {
        let kinds = generator
            .components
            .iter()
            .map(|(kind, count)| format!("{} {}", number(*count), kind))
            .collect::<Vec<_>>()
            .join(", ");
        let _ = writeln!(
            out,
            "<tr><td class=\"num\">{}</td>\
             <td><a href=\"#e-{}\" data-entity=\"{}\">{}</a><div class=\"uuid\">{}</div></td>\
             <td>{}</td></tr>",
            escape(&number(generator.count)),
            escape(&generator.id),
            escape(&generator.id),
            escape(generator.name.as_deref().unwrap_or("unnamed")),
            escape(&generator.id),
            escape(&kinds)
        );
    }
    out.push_str("</tbody>\n</table>\n</section>\n");
}

/// The entity tree's frame. What goes in it is built by the script, one branch
/// at a time, as somebody opens it.
fn tree(out: &mut String) {
    out.push_str("<section id=\"entities\">\n<h2>Entities</h2>\n");
    out.push_str(
        "<p class=\"note\">Declared entities, each with the entities its own expansion \
         derived. Everything is collapsed until you open it; long lists of siblings arrive a \
         hundred at a time and say how many are left.</p>\n",
    );
    out.push_str(
        "<div class=\"tree-controls\"><button id=\"collapse-all\" type=\"button\">\
                  collapse everything</button></div>\n",
    );
    out.push_str("<div id=\"tree\"></div>\n</section>\n");
}

fn suppressions(out: &mut String, page: &Page) {
    let _ = writeln!(
        out,
        "<section id=\"suppressions\">\n<h2>Suppressed <span class=\"count\">{}</span></h2>",
        number(page.suppressions.total)
    );
    out.push_str(
        "<p class=\"note\">Targets that were derived and then deleted: by a suppression \
         somebody wrote, or by a tiling carving a hole out of its own cells. \
         Suppressed entities are not in the tree above — this is the whole record of \
         them.</p>\n",
    );
    out.push_str("<div id=\"suppressions\"></div>\n</section>\n");
}

/// Thousands separators, so 21046 reads as a size rather than as a code.
fn number(value: usize) -> String {
    let digits = value.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (position, digit) in digits.chars().enumerate() {
        if position > 0 && (digits.len() - position) % 3 == 0 {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

/// Text from a document, made safe to write into markup.
pub fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            other => out.push(other),
        }
    }
    out
}

/// JSON made safe to sit inside a script element.
///
/// `<`, `>` and `&` can only appear inside JSON *strings* — the grammar has no
/// other use for them — so rewriting all three as their `\uXXXX` escapes leaves
/// the value identical and makes `</script>` unwritable. The two Unicode line
/// separators go the same way: they are legal in JSON and were once illegal in
/// JavaScript source, and escaping them costs nothing.
pub fn embed(json: &str) -> String {
    let mut out = String::with_capacity(json.len());
    for character in json.chars() {
        match character {
            '<' => out.push_str("\\u003c"),
            '>' => out.push_str("\\u003e"),
            '&' => out.push_str("\\u0026"),
            '\u{2028}' => out.push_str("\\u2028"),
            '\u{2029}' => out.push_str("\\u2029"),
            other => out.push(other),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    #[test]
    fn markup_in_a_document_arrives_as_text() {
        assert_eq!(
            escape("</script><b>&\"'"),
            "&lt;/script&gt;&lt;b&gt;&amp;&quot;&#39;"
        );
    }

    #[test]
    fn embedded_json_cannot_end_the_script_element() {
        let embedded = embed(r#"{"name":"</script><script>alert(1)</script>"}"#);
        assert!(!embedded.contains('<'), "{embedded}");
        assert!(!embedded.contains('>'), "{embedded}");
        // Still the same string once a JSON parser has read it.
        let value: Value = serde_json::from_str(&embedded).expect("still JSON");
        assert_eq!(value["name"], "</script><script>alert(1)</script>");
    }

    #[test]
    fn counts_read_as_sizes() {
        assert_eq!(number(0), "0");
        assert_eq!(number(999), "999");
        assert_eq!(number(1000), "1,000");
        assert_eq!(number(21046), "21,046");
    }
}
