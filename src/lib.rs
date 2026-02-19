use colored::Colorize;
use std::env;
use std::error::Error;
use std::fs;
use std::io::{BufRead, BufReader, Read};
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
    if path.is_file() {
        process_single_file(path, &config.query, config.ignore_case);
    } else if path.is_dir() {
        check_recursively(path, config.query.as_str(), config.ignore_case);
    }
    Ok(())
}

fn check_recursively(root: &Path, query: &str, ignore_case: bool) {
    if let Ok(entries) = fs::read_dir(root) {
        for entry in entries.flatten() {
            if let Ok(metadeta) = entry.metadata() {
                //detect file type , to skip sym links, sockets, pipes
                let file_type = metadeta.file_type();
                let path = entry.path();

                if file_type.is_symlink() {
                    return;
                }

                if file_type.is_dir() {
                    check_recursively(&path, query, ignore_case);
                    continue;
                }

                if file_type.is_file()
                    && let Ok(file) = fs::File::open(&path)
                {
                    let reader = BufReader::new(file);
                    let results = if ignore_case {
                        search_case_insensitive(query, reader)
                    } else {
                        search(query, reader)
                    };

                    for line in &results {
                        let res = segment_line(line, query);
                        println!("{}: {}", path.display(), res);
                    }
                }
            }
        }
    }
}

fn process_single_file(path: &Path, query: &str, ignore_case: bool) {
    if let Ok(file) = fs::File::open(path) {
        let reader = BufReader::new(file);
        let results = if ignore_case {
            search_case_insensitive(query, reader)
        } else {
            search(query, reader)
        };

        for line in results {
            let res = segment_line(&line, query);
            println!("{}: {}", path.display(), res);
        }
    }
}

pub fn search<R: Read>(query: &str, reader: BufReader<R>) -> Vec<String> {
    reader
        .lines()
        .map_while(Result::ok)
        .enumerate()
        .map(|(i, line)| format!("{}: {}", i + 1, line))
        .filter(|formatted_line| formatted_line.contains(query))
        .collect()
}

fn segment_line(line: &str, query: &str) -> String {
    let binding = line.to_lowercase();
    let indices_vec: Vec<_> = binding.match_indices(query).collect();
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

pub fn search_case_insensitive<R: Read>(query: &str, reader: BufReader<R>) -> Vec<String> {
    reader
        .lines()
        .map_while(Result::ok)
        .enumerate()
        .filter(|(_, line)| line.to_lowercase().contains(&query.to_lowercase()))
        .map(|(i, line)| format!("{}: {}", i + 1, line))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn case_sensitive() {
        let query = "duct";
        let contents = "\
rust:
safe, fast, productive.
pick three.
duct tape.";
        let reader = BufReader::new(contents.as_bytes());
        assert_eq!(
            vec!["2: safe, fast, productive.", "4: duct tape."],
            search(query, reader)
        );
    }

    #[test]
    fn case_insensitive() {
        let query = "rUsT";
        let contents = "Rust:\nsafe, fast, productive.\nTrust me.";

        let reader = BufReader::new(contents.as_bytes());
        let results = search_case_insensitive(query, reader);

        assert_eq!(vec!["1: Rust:", "3: Trust me."], results);
    }
}
