//! The main entry point for the Hash interpreter.

mod command;
mod error;

use std::{env, panic, process::exit};

use command::InteractiveCommand;
use error::InteractiveError;
use hash_driver::{driver::Driver, Compiler, CompilerBuilder};
use hash_pipeline::{
    interface::CompilerInterface,
    settings::{CompilerSettings, CompilerStageKind},
};
use hash_reporting::report::Report;
use hash_utils::{crash::crash_handler, log, logging::CompilerLogger};
use rustyline::{error::ReadlineError, Editor};

/// The logger that is used by the compiler for `log!` statements.
pub static COMPILER_LOGGER: CompilerLogger = CompilerLogger;

/// Interactive backend version
pub const VERSION: &str = env!("EXECUTABLE_VERSION");

/// Utility to print the version of the current interactive backend
#[inline(always)]
pub fn print_version() {
    println!("Version {VERSION}");
}

/// Function that is called on a graceful interpreter exit
pub fn goodbye() -> ! {
    println!("Goodbye!");
    exit(0)
}

fn main() {
    panic::set_hook(Box::new(crash_handler));
    log::set_logger(&COMPILER_LOGGER).unwrap_or_else(|_| panic!("couldn't initiate logger"));

    // @@Future: Maybe support a restricted subset of command line arguments from
    // the settings?
    let mut settings = CompilerSettings::new();

    // Configure the settings to only run up to the typechecking stage, and
    // consequently to evaluate the TIR, as this is what the interpreter
    // currently supports.
    settings.set_stage(CompilerStageKind::Analysis);
    settings.semantic_settings.eval_tir = true;

    let mut compiler = CompilerBuilder::build_with_settings(settings);

    print_version(); // Display the version on start-up
    let mut rl = Editor::<()>::new();

    loop {
        let line = rl.readline(">>> ");

        match line {
            Ok(line) => {
                rl.add_history_entry(line.as_str());
                execute(&mut compiler, line.as_str());
            }
            Err(ReadlineError::Interrupted) | Err(ReadlineError::Eof) => {
                println!("Exiting!");
                break;
            }
            Err(err) => {
                eprintln!("{}", Report::from(InteractiveError::Internal(format!("{err}"))));
            }
        }
    }
}

/// Function to process a single line of input from the REPL instance.
fn execute(compiler: &mut Driver<Compiler>, input: &str) {
    // If the entered line has no content, just skip even evaluating it.
    if input.is_empty() {
        return;
    }

    // Clear the diagnostics from the previous run.
    compiler.diagnostics_mut().clear();

    let command = InteractiveCommand::try_from(input);

    match command {
        Ok(InteractiveCommand::Quit) => goodbye(),
        Ok(InteractiveCommand::Clear) => {
            // check if this is either a unix/windows system and then execute
            // the appropriate clearing command
            if cfg!(target_os = "windows") {
                std::process::Command::new("cls").status().unwrap();
            } else {
                std::process::Command::new("clear").status().unwrap();
            }
        }
        Ok(InteractiveCommand::Version) => print_version(),
        Ok(
            ref inner @ (InteractiveCommand::Type(expr)
            | InteractiveCommand::Display(expr)
            | InteractiveCommand::Code(expr)),
        ) => {
            let settings = compiler.settings_mut();

            // if the mode is specified to emit the type `:t` of the expr or the dump tree
            // `:d`
            match inner {
                InteractiveCommand::Type(_) => {
                    // @@Hack: if display is previously set `:d`, then this interferes with this
                    // mode.
                    settings.ast_settings_mut().dump = false;
                    settings.set_stage(CompilerStageKind::Analysis)
                }
                InteractiveCommand::Display(_) => {
                    settings.ast_settings_mut().dump = true;
                    settings.set_stage(CompilerStageKind::Parse)
                }
                _ => {
                    settings.ast_settings_mut().dump = false;
                }
            }

            // Add the interactive block to the state
            compiler.run_interactive(expr.to_string());
        }
        Err(err) => {
            println!("{}", Report::from(err))
        }
    }
}
