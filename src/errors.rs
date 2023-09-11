
#[derive(std::fmt::Debug)]
pub enum MyError {
    //LogicError(&'static str),
    IoError(std::io::Error),
    NixError(nix::Error),
}

impl From<nix::Error> for MyError {
    fn from (x: nix::Error) -> Self {
        MyError::NixError(x)
    }
}

impl From<std::io::Error> for MyError {
    fn from (x: std::io::Error) -> Self {
        MyError::IoError(x)
    }
}

pub type Result<T> = std::result::Result<T, MyError>;