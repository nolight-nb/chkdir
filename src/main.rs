use std::fs::File;
use std::io::{self, BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process;
use std::sync::mpsc;

use chrono::Local;
use clap::Parser;
use md5::{Digest, Md5};
use threadpool::ThreadPool;

fn main() {
    let target_dir: TargetDir = Args::parse().target_dir();
    match target_dir.last_result() {
        LastResult::Exist(last_result) => {
            let new_result: NewResult = target_dir.new_result();
            let result_diff: Diff = new_result.merge(last_result).diff();
            new_result.write(target_dir.path);
            result_diff.summarize()
        }
        LastResult::NotExist => {
            target_dir.new_result().write(target_dir.path);
            println!("\x1B[1mThe first check is done.\x1B[0m")
        }
    }
}

/// Check file changes in the folder
#[derive(Parser)]
struct Args {
    /// The path to the folder to check
    #[arg(short, long)]
    dir: PathBuf,
}

impl Args {
    fn target_dir(&self) -> TargetDir {
        match self.dir.read_dir() {
            Ok(read_dir) => {
                let mut content: Vec<EachPath> = vec![];
                let mut result_files: Vec<PathBuf> = vec![];
                for entry in read_dir.map(|e| EachPath::new(e.unwrap().path())) {
                    match &entry {
                        EachPath::Dir(dir) => {
                            if dir.is_empty() {
                                content.push(entry)
                            } else {
                                content.append(&mut dir.visit())
                            }
                        }
                        EachPath::File(file) => {
                            if file.is_result_file() {
                                result_files.push(file.path.clone())
                            } else if file.path.file_name().unwrap().ne(".DS_Store") {
                                content.push(entry)
                            }
                        }
                    }
                }
                TargetDir {
                    path: self.dir.clone(),
                    content,
                    result_files,
                }
            }
            Err(_) => {
                eprintln!(
                    "\x1B[91;1merror:\x1B[0;1m {} \x1B[0mis not available!",
                    self.dir.display()
                );
                process::exit(65)
            }
        }
    }
}

#[derive(Clone)]
struct EachDir {
    path: PathBuf,
}

impl EachDir {
    fn is_empty(&self) -> bool {
        let count: usize = self.path.read_dir().unwrap().count();
        if count.eq(&0) {
            true
        } else if count.eq(&1) {
            self.path.join(".DS_Store").exists()
        } else {
            false
        }
    }
    fn visit(&self) -> Vec<EachPath> {
        let mut content: Vec<EachPath> = vec![];
        for entry in self
            .path
            .read_dir()
            .unwrap()
            .map(|e| EachPath::new(e.unwrap().path()))
        {
            match &entry {
                EachPath::Dir(dir) => {
                    if dir.is_empty() {
                        content.push(entry)
                    } else {
                        content.append(&mut dir.visit())
                    }
                }
                EachPath::File(file) => {
                    if file.path.file_name().unwrap().ne(".DS_Store") {
                        content.push(entry)
                    }
                }
            }
        }
        content
    }
}

#[derive(Clone)]
struct EachFile {
    path: PathBuf,
}

impl EachFile {
    fn is_result_file(&self) -> bool {
        let file_name = self.path.file_name().unwrap().to_string_lossy();
        if file_name.len().eq(&28)
            & file_name.starts_with("checkresult-")
            & file_name.ends_with(".txt")
        {
            for char in file_name.to_string()[12..24].chars() {
                if !char.is_ascii_digit() {
                    return false;
                }
            }
            true
        } else {
            false
        }
    }
}

#[derive(Clone)]
enum EachPath {
    Dir(EachDir),
    File(EachFile),
}

impl EachPath {
    fn new(path: PathBuf) -> EachPath {
        if path.is_dir() {
            EachPath::Dir(EachDir { path })
        } else if path.is_file() {
            EachPath::File(EachFile { path })
        } else {
            panic!()
        }
    }
    fn generate(&self) -> ResultItem {
        match self {
            EachPath::Dir(dir) => ResultItem {
                path: dir.path.to_string_lossy().to_string(),
                md5: "         empty_directory        ".to_string(),
            },
            EachPath::File(file) => {
                let mut file_object = File::open(file.path.clone()).unwrap();
                let mut hasher = Md5::new();
                io::copy(&mut file_object, &mut hasher).unwrap();
                ResultItem {
                    path: file.path.to_string_lossy().to_string(),
                    md5: format!("{:x}", hasher.finalize()),
                }
            }
        }
    }
}

struct ResultItem {
    path: String,
    md5: String,
}

struct TargetDir {
    path: PathBuf,
    content: Vec<EachPath>,
    result_files: Vec<PathBuf>,
}

impl TargetDir {
    fn last_result(&self) -> LastResult {
        if self.result_files.is_empty() {
            LastResult::NotExist
        } else {
            let mut tmp_num: u64 = 0;
            let mut last_result_file: PathBuf = PathBuf::new();
            for entry in &self.result_files {
                let file_num: u64 = entry.file_name().unwrap().to_str().unwrap()[12..24]
                    .to_string()
                    .parse::<u64>()
                    .unwrap();
                if file_num > tmp_num {
                    tmp_num = file_num;
                    last_result_file = entry.to_path_buf();
                }
            }
            let mut content: Vec<String> = vec![];
            for line in BufReader::new(File::open(last_result_file).unwrap()).lines() {
                content.push(line.unwrap())
            }
            LastResult::Exist(content)
        }
    }
    fn new_result(&self) -> NewResult {
        let thread_pool: ThreadPool = ThreadPool::new(num_cpus::get());
        let (sender, receiver) = mpsc::channel();
        for entry in self.content.clone() {
            let sender = sender.clone();
            thread_pool.execute(move || {
                sender.send(entry.generate()).unwrap();
            });
        }
        drop(sender);
        let mut result_items: Vec<ResultItem> = vec![];
        let total_num = self.content.len();
        let mut finish_num: u64 = 0;
        for recev in receiver.iter() {
            finish_num += 1;
            print!("Calculating MD5...\t[{}/{}]\r", finish_num, total_num);
            io::stdout().flush().unwrap();
            result_items.push(ResultItem {
                path: recev.path[self.path.to_str().unwrap().len()..].to_string(),
                md5: recev.md5,
            })
        }
        if total_num.ne(&0) {
            println!("\n");
        }
        result_items.sort_by(|a, b| a.path.cmp(&b.path));
        let mut content: Vec<String> = vec![];
        for item in result_items {
            content.push(format!("{} .{}", item.md5, item.path))
        }
        NewResult { content }
    }
}

enum LastResult {
    Exist(Vec<String>),
    NotExist,
}

#[derive(Clone)]
struct NewResult {
    content: Vec<String>,
}

impl NewResult {
    fn write(&self, path: PathBuf) {
        let mut content: String = String::new();
        for entry in &self.content {
            content.push_str(&format!("{}\n", &entry));
        }
        let mut file = File::create(path.join(format!(
            "checkresult-{}.txt",
            Local::now().format("%y%m%d%H%M%S")
        )))
        .unwrap();
        file.write_all(content.as_bytes()).unwrap();
    }
    fn merge(&self, last: Vec<String>) -> Result {
        Result {
            last,
            new: self.content.clone(),
        }
    }
}

struct Result {
    last: Vec<String>,
    new: Vec<String>,
}

impl Result {
    fn diff(&self) -> Diff {
        let mut added: Vec<String> = vec![];
        let mut deleted: Vec<String> = vec![];
        for last_item in &self.last {
            if !&self.new.contains(last_item) {
                deleted.push(last_item.to_string())
            }
        }
        for new_item in &self.new {
            if !&self.last.contains(new_item) {
                added.push(new_item.to_string())
            }
        }
        if added.is_empty() & deleted.is_empty() {
            println!("\x1B[1mNo change.\x1B[0m");
            process::exit(0)
        }
        Diff { added, deleted }
    }
}

struct Diff {
    added: Vec<String>,
    deleted: Vec<String>,
}

impl Diff {
    fn summarize(&self) {
        if self.added.is_empty() & self.deleted.is_empty() {
            println!("\x1B[1mNo change.\x1B[0m");
            process::exit(0)
        }
        if !self.added.is_empty() {
            println!("\x1B[92;1mNewly added:\x1B[0m");
        }
        self.added.iter().for_each(|i| println!("{}", i));
        if !self.added.is_empty() & !self.deleted.is_empty() {
            println!()
        }
        if !self.deleted.is_empty() {
            println!("\x1B[96;1mRemoved:\x1B[0m");
        }
        self.deleted.iter().for_each(|i| println!("{}", i));
    }
}
