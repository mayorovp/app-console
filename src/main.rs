use std::env;
use std::ffi::{ OsString, CString };
use std::os::unix::ffi::OsStrExt;
use std::os::unix::io::{ FromRawFd };
use nix::unistd;

mod errors;
mod server;

fn os_to_cstring(x : OsString) -> CString {
    CString::new(x.as_bytes()).unwrap()
}

fn main() {
    let socket_path = env::args_os().nth(1).expect("Socket path required");
    let child_file = os_to_cstring(env::args_os().nth(2).expect("Application command required"));
    let child_args : Vec<CString> = env::args_os().skip(2).map(os_to_cstring).collect();

    let run_program = || -> errors::Result<i32> {
        let (pipe_in, pipe_out) = unistd::pipe()?;

        let exit_code = match unsafe { unistd::fork() }? {
            unistd::ForkResult::Child => {
                unistd::close(pipe_out).expect("Child close failed");
                unistd::dup2(pipe_in, 0).expect("Child dup2 failed");
                unistd::execvp(child_file.as_c_str(), &child_args).expect("Child execvp failed");
                0 // unreachable
            }

            unistd::ForkResult::Parent { child: child_pid } => {
                unistd::close(pipe_in)?;

                let console = unsafe { std::fs::File::from_raw_fd(pipe_out) };
                server::run_server(child_pid, console, &socket_path)?
            }
        };

        Ok(exit_code)
    };

    let exit_code = run_program().unwrap();
    std::process::exit(exit_code);
}