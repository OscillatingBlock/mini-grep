use crate::pool::ThreadPool;
use std::error::Error;
use std::fs;
use std::io::{BufRead, BufReader, Read};
use std::io::{Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Instant;
use std::{env, thread};

const BATCH_SIZE: usize = 1000;
pub struct Config {
    pub query: String,
    pub file_path: String,
    pub ignore_case: bool,
}

struct Job {
    query: Arc<String>,
    paths: Vec<PathBuf>,
    ignore_case: bool,
}

pub mod pool;

struct SearchResult {
    file_path: String,
    segmented_lines: Vec<String>,
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
    let start = Instant::now();
    let path = Path::new(&config.file_path);
    let query = Arc::new(config.query);

    //  Create channels (Sender is natively Cloneable)
    let (work_tx, work_rx) = mpsc::channel::<Job>();
    let (res_tx, res_rx) = mpsc::channel::<SearchResult>();

    let pool = ThreadPool::new(8);

    //  Spawn Worker Manager (Give it a clone of res_tx)
    let res_tx_child = res_tx.clone();
    thread::spawn(move || {
        spawn_worker_thread(pool, work_rx, res_tx_child);
    });

    if path.is_file() {
        process_single_file(path, query.clone(), config.ignore_case);
    } else if path.is_dir() {
        let path_to_move = PathBuf::from(&config.file_path);
        let query_child = Arc::clone(&query);

        //  Walker Logic in Main Thread (To ensure we don't exit early)
        let mut buffer = Vec::with_capacity(100);
        check_recursively(
            path_to_move,
            query_child.clone(),
            config.ignore_case,
            work_tx.clone(),
            &mut buffer,
        );

        //  Final Flush for Batching
        if !buffer.is_empty() {
            work_tx
                .send(Job {
                    query: query_child,
                    paths: buffer,
                    ignore_case: config.ignore_case,
                })
                .ok();
        }
    }

    //  Drop the main thread's copies
    // This allows work_rx to close once the walker is done,
    // and res_rx to close once the workers are done.
    //
    // safe to drop because we are just closing main threads's connection to the channel
    // channel wont be closed since other threads have the copy of transmitting end still
    drop(work_tx);
    drop(res_tx);

    println!("starting results loop");
    while let Ok(res) = res_rx.recv() {
        println!("\x1b[1;32m{}:\x1b[0m", res.file_path);

        for segment_line in res.segmented_lines {
            println!("  {}", segment_line);
        }
        println!();
    }

    println!("Search finished in {:?}", start.elapsed());
    Ok(())
}

fn spawn_worker_thread(
    pool: ThreadPool,
    work_rx: Receiver<Job>,
    res_tx: mpsc::Sender<SearchResult>,
) {
    while let Ok(job) = work_rx.recv() {
        let res_tx = res_tx.clone();
        pool.execute(move || {
            for path in job.paths {
                if let Ok(file) = fs::File::open(&path) {
                    // Search
                    let mut buffer = [0; 1024];
                    let mut reader = BufReader::new(file);

                    //  Binary Check
                    if let Ok(n) = reader.read(&mut buffer)
                        && buffer[..n].contains(&0)
                    {
                        continue;
                    }

                    let _ = reader.seek(SeekFrom::Start(0));

                    let matches = if job.ignore_case {
                        search_case_insensitive(Arc::clone(&job.query), reader)
                    } else {
                        search(Arc::clone(&job.query), reader)
                    };

                    //  Send only if matches were found
                    if !matches.is_empty() {
                        let segmented_lines = matches
                            .into_iter()
                            .map(|line| segment_line(&line, &job.query))
                            .collect();

                        let result = SearchResult {
                            file_path: path.display().to_string(),
                            segmented_lines,
                        };

                        let _ = res_tx.send(result);
                    }
                }
            }
        });
    }
}

fn check_recursively(
    root: PathBuf,
    query: Arc<String>,
    ignore_case: bool,
    work_tx: Sender<Job>,
    buffer: &mut Vec<PathBuf>,
) {
    if let Ok(entries) = fs::read_dir(root) {
        for entry in entries.flatten() {
            let work_tx = work_tx.clone();
            if let Ok(metadeta) = entry.metadata() {
                //detect file type , to skip sym links, sockets, pipes
                let file_type = metadeta.file_type();
                let path = entry.path();

                //ignore hidden files and directories
                if path.display().to_string().starts_with(".") {
                    continue;
                }

                if file_type.is_symlink() {
                    continue;
                } else if file_type.is_dir() {
                    check_recursively(path, Arc::clone(&query), ignore_case, work_tx, buffer);
                    continue;
                } else if file_type.is_file() {
                    buffer.push(path); // YOU MUST HAVE THIS LINE

                    if buffer.len() >= BATCH_SIZE {
                        let job = Job {
                            query: Arc::clone(&query),
                            paths: buffer.clone(),
                            ignore_case,
                        };
                        let _ = work_tx.send(job);
                        buffer.clear();
                    }
                }
            }
        }
    }
}

fn process_single_file(path: &Path, query: Arc<String>, ignore_case: bool) {
    if let Ok(file) = fs::File::open(path) {
        let reader = BufReader::new(file);
        let results = if ignore_case {
            search_case_insensitive(query, reader)
        } else {
            search(query, reader)
        };

        for line in results {
            // let res = segment_line(&line, query);
            println!("{}: {}", path.display(), line);
        }
    }
}

pub fn search<R: Read>(query: Arc<String>, reader: BufReader<R>) -> Vec<String> {
    reader
        .lines()
        .map_while(Result::ok)
        .enumerate()
        .map(|(i, line)| format!("{}: {}", i + 1, line))
        .filter(|formatted_line| formatted_line.contains(query.as_str()))
        .collect()
}

fn segment_line(line: &str, query: &str) -> String {
    let binding = line.to_lowercase();
    let indices_vec: Vec<_> = binding.match_indices(query).collect();

    let mut segmented_line = String::with_capacity(line.len() + 32);
    let mut last_pos = 0;

    for (start, _) in indices_vec {
        let end = start + query.len();
        segmented_line.push_str(&line[last_pos..start]);

        //ANSI code for red start
        segmented_line.push_str("\x1b[31m"); // Red start
        segmented_line.push_str(&line[start..end]);
        //red end
        segmented_line.push_str("\x1b[0m"); // Reset

        last_pos = end;
    }

    segmented_line.push_str(&line[last_pos..]);
    segmented_line
}

pub fn search_case_insensitive<R: Read>(query: Arc<String>, reader: BufReader<R>) -> Vec<String> {
    let query = &query.to_lowercase();
    reader
        .lines()
        .map_while(Result::ok)
        .enumerate()
        .filter(|(_, line)| line.to_lowercase().contains(query))
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
            search(Arc::new(query.to_string()), reader)
        );
    }

    #[test]
    fn case_insensitive() {
        let query = "rUsT";
        let contents = "Rust:\nsafe, fast, productive.\nTrust me.";

        let reader = BufReader::new(contents.as_bytes());
        let results = search_case_insensitive(Arc::new(query.to_string()), reader);

        assert_eq!(vec!["1: Rust:", "3: Trust me."], results);
    }
}
