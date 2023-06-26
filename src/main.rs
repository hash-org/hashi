//! The main entry point for the Hash interpreter.

mod command;
mod error;

use std::{env, process::exit};

use command::InteractiveCommand;
use error::InteractiveError;
use hash_ast::node_map::InteractiveBlock;
use hash_pipeline::{
    interface::{CompilerInterface, CompilerOutputStream},
    settings::{CompilerSettings, CompilerStageKind},
    workspace::Workspace,
    Compiler,
};
use hash_reporting::{report::Report, writer::ReportWriter};
use hash_session::{emit_fatal_error, make_stages, CompilerSession};
use hash_source::SourceMap;
use rustyline::{error::ReadlineError, Editor};

/// Interactive backend version
pub const VERSION: &str = env!("EXECUTABLE_VERSION");

/// Utility to print the version of the current interactive backend
#[inline(always)]
pub fn print_version() {
    println!("Version {VERSION}");
}

/// Function that is called on a graceful interpreter exit
pub fn goodbye() {
    println!("Goodbye!");
    exit(0)
}

/// Perform some task that might fail and if it does, report the error and exit,
/// otherwise return the result of the task.
fn handle_error<T, E: Into<Report>>(sources: &SourceMap, f: impl FnOnce() -> Result<T, E>) -> T {
    match f() {
        Ok(value) => value,
        Err(err) => emit_fatal_error(err, sources),
    }
}

fn main() {
    // @@Hack: we have to create a dummy source map here so that we can use it
    // to report errors in the case that the compiler fails to start up. After the
    // workspace is initiated, it is replaced with the real source map.
    let source_map = SourceMap::new();
    let settings = CompilerSettings::new();

    // We want to figure out the entry point of the compiler by checking if the
    // compiler has been specified to run in a specific mode.
    let _entry_point = handle_error(&source_map, || settings.entry_point().transpose());
    let workspace = handle_error(&source_map, || Workspace::new(&settings));

    // We need at least 2 workers for the parsing loop in order so that the job
    // queue can run within a worker and any other jobs can run inside another
    // worker or workers.
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(settings.worker_count + 1)
        .thread_name(|id| format!("compiler-worker-{id}"))
        .build()
        .unwrap();

    let session = CompilerSession::new(
        workspace,
        pool,
        settings,
        || CompilerOutputStream::Stderr(std::io::stderr()),
        || CompilerOutputStream::Stdout(std::io::stdout()),
    );
    let mut compiler = Compiler::new(make_stages());
    let mut compiler_state = compiler.bootstrap(session);

    print_version(); // Display the version on start-up
    let mut rl = Editor::<()>::new();

    loop {
        let line = rl.readline(">>> ");

        match line {
            Ok(line) => {
                rl.add_history_entry(line.as_str());
                compiler_state = execute(line.as_str(), &mut compiler, compiler_state);
            }
            Err(ReadlineError::Interrupted) | Err(ReadlineError::Eof) => {
                println!("Exiting!");
                break;
            }
            Err(err) => {
                eprintln!(
                    "{}",
                    ReportWriter::new(
                        vec![InteractiveError::Internal(format!("{err}")).into()],
                        compiler_state.source_map()
                    )
                );
            }
        }
    }
}

/// Function to process a single line of input from the REPL instance.
fn execute<I: CompilerInterface>(input: &str, compiler: &mut Compiler<I>, mut ctx: I) -> I {
    // If the entered line has no content, just skip even evaluating it.
    if input.is_empty() {
        return ctx;
    }

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
            // Add the interactive block to the state
            let interactive_id =
                ctx.add_interactive_block(expr.to_string(), InteractiveBlock::new());
            let settings = ctx.settings_mut();

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

            // We don't want the old diagnostics
            // @@Refactor: we don't want to leak the diagnostics here..
            ctx.diagnostics_mut().clear();
            let new_state = compiler.run(interactive_id, ctx);
            return new_state;
        }
        Err(err) => {
            println!("{}", ReportWriter::new(vec![err.into()], ctx.source_map()))
        }
    }

    ctx
}
