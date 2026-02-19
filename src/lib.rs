use colored::Colorize;
use std::env;
use std::error::Error;
use std::fs;
use std::path::Path;

pub struct Config {
    pub query: String,
    pub file_path: String,
    pub ignore_case: bool,
}

impl Config {
    pub fn build(mut args: impl Iterator<Item = String>) -> Result<Config, &'static str> {
        args.next();

        let query = match args.next() {
            Some(arg) => arg,
            None => return Err("Didn't get a query string"),
        };

        let file_path = match args.next() {
            Some(arg) => arg,
            None => return Err("Didn't get a file path"),
        };

        let ignore_case = env::var("IGNORE_CASE").is_ok();

        Ok(Config {
            query,
            file_path,
            ignore_case,
        })
    }
}

pub fn run(config: Config) -> Result<(), Box<dyn Error>> {
    let path = Path::new(&config.file_path);
    check_recursively(path, config.query.as_str());
    Ok(())
}

fn check_recursively(root: &Path, query: &str) {
    if let Ok(entries) = fs::read_dir(root) {
        for entry in entries.flatten() {
            if let Ok(metadeta) = entry.metadata() {
                //detect file type , to skip sym links, sockets, pipes
                let file_type = metadeta.file_type();
                let path = entry.path();

                if file_type.is_dir() {
                    check_recursively(&path, query);
                    continue;
                }

                if file_type.is_file()
                    && let Ok(contents) = fs::read_to_string(&path)
                {
                    let results = search(query, &contents);

                    for line in &results {
                        let res = segment_line(line, query);
                        println!("{}: {}", path.display(), res);
                    }
                }
            }
        }
    }
}

pub fn search(query: &str, contents: &str) -> Vec<String> {
    contents
        .lines()
        .enumerate()
        .map(|(i, line)| format!("{}: {}", i + 1, line))
        .filter(|formatted_line| formatted_line.contains(query))
        .collect()
}

fn segment_line(line: &str, query: &str) -> String {
    let indices_vec: Vec<_> = line.match_indices(query).collect();
    let mut segmented_line = String::from("");
    let mut last_pos = 0;

    for elems in indices_vec {
        segmented_line.push_str(&line[last_pos..elems.0]);
        segmented_line.push_str(&line[elems.0..elems.0 + query.len()].red().to_string());
        last_pos = elems.0 + query.len();
    }
    segmented_line.push_str(&line[last_pos..]);
    segmented_line
}

pub fn search_case_insensitive<'a>(query: &str, contents: &'a str) -> Vec<&'a str> {
    let query = query.to_lowercase();
    let mut results = Vec::new();

    for line in contents.lines() {
        if line.to_lowercase().contains(&query) {
            results.push(line);
        }
    }

    results
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn case_sensitive() {
        let query = "duct";
        let contents = "\
Rust:
safe, fast, productive.
Pick three.
Duct tape.";

        assert_eq!(vec!["safe, fast, productive."], search(query, contents));
    }

    #[test]
    fn case_insensitive() {
        let query = "rUsT";
        let contents = "\
Rust:
safe, fast, productive.
Pick three.
Trust me.";

        assert_eq!(
            vec!["Rust:", "Trust me."],
            search_case_insensitive(query, contents)
        );
    }
}
