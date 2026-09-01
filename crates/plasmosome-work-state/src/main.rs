use plasmosome_work_state::{contract::parse_contract_request, run_contract};

fn main() {
    let request =
        parse_contract_request(std::env::args().skip(1)).unwrap_or_else(|code| fail(&code, 2));
    match run_contract(&request) {
        Ok(result) => println!("{}", serde_json::to_string(&result).unwrap()),
        Err(result) => {
            println!("{}", serde_json::to_string(&result).unwrap());
            fail(
                &result.code,
                if result.code == "cutover_blocked" {
                    1
                } else {
                    2
                },
            );
        }
    }
}

fn fail(message: &str, code: i32) -> ! {
    eprintln!("{message}");
    std::process::exit(code)
}
