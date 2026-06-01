#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    match code_pet_lib::cli::try_handle_cli() {
        Ok(true) => return,
        Ok(false) => {}
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(1);
        }
    }
    code_pet_lib::run();
}
