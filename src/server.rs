use std::fs::File;
use std::sync::{ Mutex, Arc };
use std::path::Path;
use std::thread;
use std::io::{ BufReader, BufRead, Write };
use std::ffi::{ OsStr };
use std::os::unix::net::{ UnixListener, UnixStream };
use std::os::unix::io::{ AsRawFd };
use nix::sys::{signal, signalfd, wait, epoll};
use nix::unistd;
use libc;
use super::errors::Result;

struct ActiveConnection {
    stream: Arc<UnixStream>,
    thread: std::thread::JoinHandle<()>,
}

struct ActiveConnectionList {
    list: Arc<Mutex<Vec<ActiveConnection>>>
}

impl ActiveConnectionList {
    fn new() -> ActiveConnectionList {
        ActiveConnectionList { 
            list: Arc::new(Mutex::new(Vec::<ActiveConnection>::new()))
        }
    }

    fn add(&self, stream: UnixStream, handler: impl FnOnce(&UnixStream) -> () + Send + 'static) {
        let stream = Arc::new(stream);
        let mut lock = self.list.lock().expect("poisoned lock");
        
        let stream_copy = stream.clone();
        let list_copy = self.list.clone();
        let thread = thread::spawn(move || {
            handler(&stream_copy);

            let mut lock = list_copy.lock().expect("poisoned lock");
            match lock.iter().position(|x| Arc::ptr_eq(&x.stream, &stream_copy)) {
                Some(index) => {
                    lock.remove(index);
                },
                None => (),
            };
        });

        lock.push(ActiveConnection { stream, thread });
    }
}

impl Drop for ActiveConnectionList {
    fn drop(&mut self) {
        let connections : Vec<ActiveConnection> = self.list.lock().expect("poisoned lock").drain(..).collect();        
        for connection in connections {
            if let Err(err) = connection.stream.shutdown(std::net::Shutdown::Both) {
                println!("Connection shutdown failed, code = {}", err);
            }
            if let Err(err) = connection.thread.join() {
                println!("Connection thread panic, code = {:?}", err);
            }
        }
    }
}

pub fn run_server(child_pid: unistd::Pid, console: File, socket_path: &OsStr) -> Result<i32> {
    let mut masked_signals = signal::SigSet::empty();
    masked_signals.add(signal::Signal::SIGCHLD);
    signal::pthread_sigmask(signal::SigmaskHow::SIG_BLOCK, Option::Some(&masked_signals), Option::None)?;

    let mut signal_fd = signalfd::SignalFd::new(&masked_signals)?;
    let listener = UnixListener::bind(Path::new(socket_path))?;

    let epoll_fd = epoll::epoll_create()?;
    epoll::epoll_ctl(epoll_fd, epoll::EpollOp::EpollCtlAdd, signal_fd.as_raw_fd(), Option::Some(&mut epoll::EpollEvent::new(epoll::EpollFlags::EPOLLIN, 0)))?;
    epoll::epoll_ctl(epoll_fd, epoll::EpollOp::EpollCtlAdd, listener.as_raw_fd(), Option::Some(&mut epoll::EpollEvent::new(epoll::EpollFlags::EPOLLIN, 1)))?;

    match wait::waitpid(child_pid, Option::Some(wait::WaitPidFlag::WNOHANG))? {
        wait::WaitStatus::Exited(_, exit_code) => return Ok(exit_code),
        wait::WaitStatus::Signaled(_, _, _) => return Ok(-1),
        wait::WaitStatus::StillAlive => (),
        _ => panic!("Unknown waitpid result"),
    };

    let console = Arc::new(Mutex::new(console));
    let active_connections = ActiveConnectionList::new();

    let exit_code = loop {
        let mut events = [ epoll::EpollEvent::empty() ];
        if epoll::epoll_wait(epoll_fd, &mut events, -1)? > 0 {
            match events[0].data() {
                1 => {
                    let (stream, _) = listener.accept()?;

                    let console_copy = console.clone();
                    active_connections.add(stream, move |stream| {
                        run_connection(&stream, &console_copy);
                    })
                }

                0 => {
                    if let Some(info) = signal_fd.read_signal()? {
                        if info.ssi_signo == signal::Signal::SIGCHLD as u32 {
                            if info.ssi_code == libc::CLD_EXITED {
                                break info.ssi_status;
                            } else {
                                break -1;
                            }
                        }
                    }
                }

                _ => panic!("Unexpected epoll_wait result")
            }
        }
    };

    drop(active_connections);
    Ok(exit_code)
}

fn run_connection(stream: &UnixStream, console: &Mutex<File>) {
    let mut reader = BufReader::new(stream);
    let mut line = String::default();

    loop {
        let command = match reader.read_line(&mut line) {
            Ok(0) => return (),
            Err(err) => {
                println!("Connection read error, code = {}", err);
                return ();
            }
            Ok(_) => &line,
        };

        match console.lock().expect("poisoned lock").write(command.as_bytes()) {
            Ok(_) => (),
            Err(err) => {
                if err.kind() == std::io::ErrorKind::BrokenPipe {
                    return ()
                }

                println!("Console write error, code = {}", err);
                return ();
            }
        }
    }
}