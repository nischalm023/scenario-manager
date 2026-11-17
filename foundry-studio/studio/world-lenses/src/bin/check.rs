//! `foundry-worldlens-check` — run a pattern library over a document and say what it
//! found.
//!
//! This is the smallest useful consumer of the engine, and it exists so that a
//! person can point a pattern at a real document and read the answer without
//! writing a program. It prints one count per pattern first, because a count is
//! what says whether a pattern means what its author thought — a
//! `placed_station` that finds eleven where the site has nineteen is a wrong
//! pattern or a surprising document, and either way the number is what shows
//! it. The bindings follow, capped so the counts stay readable, and `--explain`
//! turns the whole library on one entity: which patterns it takes part in, and
//! for the rest, the clause that let it go.
//!
//! `--lens NAME` asks the other question — not "what matches" but "what are the
//! things" — and prints one object per line as JSON, which is what a consumer
//! would receive. It prints nothing else, so the output pipes into `jq` and
//! into a diff of two documents. `--array` prints the same objects as the one
//! canonical line `worldlens::canonical_projection` gives, which is what a
//! server and a client compare a whole answer by: a line-by-line diff shows
//! which object moved, and the single line says whether the two answers are the
//! same answer at all.

use site_model::EntityId;
use std::path::PathBuf;
use std::process::ExitCode;
use worldlens::{canonical_projection, Library, Matcher};

const USAGE: &str = "usage: foundry-worldlens-check [--explain <entity-id>] [--limit <n>] [--lens <name>] [--array] \
                     [--count] <scenario-or-document> <patterns.lens>";

fn main() -> ExitCode {
    let mut positional: Vec<PathBuf> = Vec::new();
    let mut explain: Option<String> = None;
    let mut wanted_lens: Option<String> = None;
    let mut counting = false;
    let mut as_array = false;
    // Ten is enough to see the shape of a pattern's answers; `--limit 0` prints
    // every one, which is what a person diffing two documents wants.
    let mut limit = 10usize;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--explain" => match args.next() {
                Some(id) => explain = Some(id),
                None => return refuse("--explain needs an entity identity"),
            },
            "--lens" => match args.next() {
                Some(name) => wanted_lens = Some(name),
                None => return refuse("--lens needs the name of a lens in the pattern file"),
            },
            "--array" => as_array = true,
            "--count" => counting = true,
            "--limit" => match args.next().map(|value| value.parse::<usize>()) {
                Some(Ok(value)) => limit = value,
                _ => return refuse("--limit needs a count, or 0 for every binding"),
            },
            flag if flag.starts_with("--") => return refuse(&format!("unknown option {flag}")),
            _ => positional.push(PathBuf::from(arg)),
        }
    }
    let [document, patterns] = positional.as_slice() else {
        return refuse(USAGE);
    };
    // Each of these would otherwise be a flag that quietly did nothing, and a
    // person comparing two documents would read the wrong output as the answer.
    if as_array && wanted_lens.is_none() {
        return refuse("--array is one lens's objects as one line, so it needs --lens");
    }
    if as_array && counting {
        return refuse("--array asks what the objects are and --count how many there are; ask one of them");
    }

    let text = match std::fs::read_to_string(patterns) {
        Ok(text) => text,
        Err(error) => return refuse(&format!("cannot read {}: {error}", patterns.display())),
    };
    let library = match Library::parse(&text) {
        Ok(library) => library,
        Err(error) => return refuse(&format!("{}: {error}", patterns.display())),
    };
    let (_layers, world) = match site_model::load(document) {
        Ok(loaded) => loaded,
        Err(error) => return refuse(&format!("{}: {error}", document.display())),
    };

    let matcher = Matcher::new(&world.entities);

    // A lens asks a different question from a pattern — not what matches, but
    // what the things are — so it answers on its own and prints nothing else.
    if let Some(name) = &wanted_lens {
        let Some(lens) = library.lens(name) else {
            let known: Vec<&str> = library.lenses().map(|lens| lens.name()).collect();
            return refuse(&format!("`{name}` is no lens of {}; it holds {}", patterns.display(), known.join(", ")));
        };
        let objects = matcher.project(lens);
        if counting {
            println!("{}", objects.len());
            return ExitCode::SUCCESS;
        }
        if as_array {
            println!("{}", canonical_projection(&objects));
            return ExitCode::SUCCESS;
        }
        for object in &objects {
            println!("{}", site_model::canonical::to_canonical_string(&object.to_json()));
        }
        return ExitCode::SUCCESS;
    }
    // Without a name, `--count` is the same question of every lens at once.
    if counting {
        for lens in library.lenses() {
            println!("{:<28}{:>8}", lens.name(), matcher.project(lens).len());
        }
        return ExitCode::SUCCESS;
    }

    println!("document: {}", document.display());
    println!("patterns: {} ({} patterns, {} lenses)", patterns.display(), library.len(), library.lenses().count());
    println!(
        "entities: {} ({} declared, {} derived)",
        world.entities.len(),
        world.entities.len() - world.derived_count(),
        world.derived_count()
    );

    let found: Vec<_> = library.patterns().map(|pattern| (pattern, matcher.matches(pattern))).collect();
    println!("matches:");
    for (pattern, bindings) in &found {
        println!("  {:<28}{:>8}", pattern.name(), bindings.len());
    }
    // A zero is the one answer that says nothing about whether the pattern is
    // right, so a zero is where it is worth naming the authoring mistake that
    // produces one: a field path written with its own component's name in
    // front of it reads a key nested inside the component and finds none.
    for (pattern, bindings) in &found {
        if !bindings.is_empty() {
            continue;
        }
        for repeated in pattern.repeated_component_paths() {
            eprintln!(
                "warning: {} matched zero, and `{}({} …)` repeats its component's name; \
                 field paths are relative — did you mean `{}({} …)`?",
                pattern.name(),
                repeated.component,
                repeated.written,
                repeated.component,
                repeated.meant
            );
        }
    }
    println!("bindings:");
    for (pattern, bindings) in &found {
        println!("  {}:", pattern.name());
        let shown = if limit == 0 { bindings.len() } else { limit.min(bindings.len()) };
        for binding in &bindings[..shown] {
            println!("    {binding}");
        }
        if shown < bindings.len() {
            println!("    … {} more", bindings.len() - shown);
        }
    }

    let Some(text) = explain else {
        return ExitCode::SUCCESS;
    };
    let Ok(entity) = text.parse::<EntityId>() else {
        return refuse(&format!("`{text}` is no entity identity: a UUID, or a UUID and a `/`-joined key"));
    };
    println!("explain: {entity}");
    let membership = matcher.membership(&library, &entity);
    println!("  belongs to:");
    if membership.is_empty() {
        println!("    nothing in this library");
    }
    for entry in &membership {
        println!("    {} as `{}`", entry.pattern, entry.variable);
    }
    for pattern in library.patterns() {
        let explanation = matcher.explain(pattern, &entity);
        // A pattern the entity already takes part in as the anchor has nothing
        // to explain; the near misses are what a person is looking at this for.
        if explanation.holds() {
            continue;
        }
        print!("{explanation}");
    }
    ExitCode::SUCCESS
}

fn refuse(message: &str) -> ExitCode {
    eprintln!("{message}");
    ExitCode::from(2)
}
