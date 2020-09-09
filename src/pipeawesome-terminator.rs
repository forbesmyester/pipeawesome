use clap::{Arg as ClapArg, App as ClapApp};
use std::io::{BufReader, BufRead, Write};
use std::collections::HashSet;
use regex::Regex;
use std::time::Instant;
use std::sync::mpsc::{sync_channel, SyncSender, SendError, Receiver};

#[derive(Debug)]
struct Opts {
    require_prequel: String,
    require_end: Option<String>,
    line_regex: String,
    line_replace: String,
    grace_period: u64,
}

fn get_opts() -> Opts {

    let matches = ClapApp::new("pipeawesome-terminator")
        .version("0.0.0")
        .author("Matthew Forrester")
        .about("Terminates after all required have been seen")
        .arg(ClapArg::with_name("require-item-prequel")
            .required(true)
            .takes_value(true)
            .short("r")
            .long("require-item-prequel")
            .help("If a line starts with this, it will be added as a requirement (depending on REPLACE_FOR_REQUIREMENET)")
        )
        .arg(ClapArg::with_name("require-end")
            .required(false)
            .takes_value(true)
            .short("e")
            .long("require-end")
            .help("If a line starts with this, it signifies all requirements have been sent")
        )
        .arg(ClapArg::with_name("line-regex")
            .help("lines that match this will be processed along with LINE_REPLACE")
            .required(false)
            .takes_value(true)
            .short("l")
            .long("line-regex")
        )
        .arg(ClapArg::with_name("line-replace")
            .help("what to replace LINE_REGEX with")
            .required(false)
            .takes_value(true)
            .short("p")
            .long("line-replace")
        )
        .arg(ClapArg::with_name("grace-period")
            .help("grace period (seconds) to get more requirements after all requirements are met.")
            .takes_value(true)
            .short("g")
            .long("grace-period")
            .env("PIPEAWESOME_TERMINATOR_GRACE_PERIOD")
            .validator(|s| {
                let re = Regex::new(r"^[0-9]+$").unwrap();
                if !re.is_match(&s) {
                    return Err(format!("GRACE_PERIOD '{}' is invalid.", s));
                }
                Ok(())
            })
        ).get_matches();

    Opts {
        require_prequel: match matches.value_of("require-item-prequel") {
            None => "".to_owned(),
            Some(s) => s.to_string(),
        },
        require_end: match matches.value_of("require-end") {
            None => None,
            Some(s) => Some(s.to_string()),
        },
        line_regex: match matches.value_of("line-regex") {
            None => "".to_owned(),
            Some(s) => s.to_string(),
        },
        line_replace: match matches.value_of("line-replace") {
            None => "".to_owned(),
            Some(s) => s.to_string(),
        },
        grace_period: match matches.value_of("grace-period") {
            None => 0,
            Some(s) => {
                match s.parse() {
                    Ok(n) => n,
                    Err(_) => { panic!("GRACE_PERIOD '{}' is invalid", s); }
                }
            }
        },
    }

}


fn main() {

    let opts = get_opts();
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    let mut br = BufReader::new(stdin);

    type Line = Option<String>;
    let (tx, rx): (SyncSender<Line>, Receiver<Line>) = sync_channel(8);

    std::thread::spawn(move || {
        loop {
            let mut s = String::new();
            match br.read_line(&mut s) {
                Ok(count) => {
                    let v = if count == 0 { None } else { Some(s) };
                    match tx.send(v) {
                        Ok(_) => (),
                        Err(SendError(vv)) => {
                            eprintln!("Error could not send '{:?}'", &vv);
                            std::process::exit(1);
                        }
                    }
                    if count == 0 {
                        return;
                    }
                }
                Err(e) => {
                    eprintln!("Error reading: {:?}", e);
                    std::process::exit(1);
                }
            }
        }
    });

    let re: Regex = Regex::new(&opts.line_regex).unwrap();
    let mut seen: Vec<( Instant, String )> = Vec::with_capacity(1024);

    let mut requirements: HashSet<String> = HashSet::new();
    let mut last_data = Instant::now();
    let mut require_end_seen = false;

    loop {
        let time_since = Instant::now().duration_since(last_data);
        if
            requirements.is_empty() &&
            (require_end_seen || (time_since.as_secs() > opts.grace_period))
        {
            std::process::exit(0);
        }
        match rx.try_recv() {
            Ok(Some(line)) if line.starts_with(&opts.require_prequel) => {
                // println!("{} = {:?} = {:?} = {:?}\n\n", &line, &requirements, &seen, &require_end_seen);
                let (_, to_ins_cow) = line.split_at(opts.require_prequel.len());
                let mut to_ins = to_ins_cow.to_string();
                to_ins.truncate(to_ins.len() - 1);
                requirements.insert(to_ins.to_string());
                for (_, s) in seen.iter().rev() {
                    if re.is_match(s) {
                        let x = re.replace(s, &opts.line_replace as &str);
                        requirements.remove(&x.to_string());
                    }
                }
                last_data = std::time::Instant::now();
            },
            Ok(Some(line)) if opts.require_end.as_ref().map(|e| line.starts_with(e)).unwrap_or(false) => {
                // println!("{} = {:?} = {:?} = {:?}\n\n", &line, &requirements, &seen, &require_end_seen);
                require_end_seen = true;
            },
            Ok(Some(mut line)) => {
                // println!("{} = {:?} = {:?} = {:?}\n\n", &line, &requirements, &seen, &require_end_seen);
                match stdout.write_all(line.as_bytes()).and_then(|_x| stdout.flush()) {
                    Ok(_) => (),
                    Err(e) => {
                        eprintln!("Error writing: {:?}", e);
                        std::process::exit(1);
                    },
                }
                line.truncate(line.len() - 1);
                seen.push((Instant::now(), line));
                let mut remove_from: Option<usize> = None;
                for (index, (inst, s)) in seen.iter().enumerate() {
                    let age = Instant::now().duration_since(*inst);
                    if age.as_secs() > opts.grace_period {
                        remove_from = Some(index);
                    }
                    if re.is_match(s) {
                        let x = re.replace(s, &opts.line_replace as &str);
                        requirements.remove(&x.to_string());
                    }
                }
                if let Some(rf) = remove_from {
                    for _i in 0..rf {
                        seen.remove(0);
                    }
                }
                last_data = std::time::Instant::now();
            },
            Ok(None) => {
                // print!("NONE\n");
                match stdout.flush() {
                    Ok(_) => (),
                    Err(e) => {
                        eprintln!("Error writing: {:?}", e);
                        std::process::exit(1);
                    },
                }
                std::process::exit(0);
            }
            Err(_) => {
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
        }
    }

}
