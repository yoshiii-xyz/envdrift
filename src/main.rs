use std::process;

fn main() {
    let code = match envdrift::entry(std::env::args_os()) {
        Ok(code) => code,
        Err(error) => {
            eprintln!("envdrift: {error}");
            2
        }
    };
    process::exit(code);
}
