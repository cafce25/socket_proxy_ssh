use std::{
    os::fd::{BorrowedFd, FromRawFd},
    time::Duration,
};

use nix::{
    fcntl::{self, FcntlArg, OFlag},
    libc::O_NONBLOCK,
};
use systemd::daemon::{Listening, SocketType};
use tokio::{join, net::UnixSocket, process::Command};

type Result<T, E = Box<dyn std::error::Error>> = std::result::Result<T, E>;
#[tokio::main]
async fn main() -> Result<()> {
    let mut listeners = systemd::daemon::listen_fds(true)?
        .iter()
        .map(|fd| {
            let socket = if systemd::daemon::is_socket(
                fd,
                None,
                Some(SocketType::Stream),
                Listening::IsListening,
            )? {
                {
                    let fd = unsafe { BorrowedFd::borrow_raw(fd) };
                    let flags = fcntl::fcntl(fd, FcntlArg::F_GETFL)?;
                    fcntl::fcntl(
                        fd,
                        FcntlArg::F_SETFL(OFlag::from_bits(flags | O_NONBLOCK).unwrap()),
                    )?;
                }
                if systemd::daemon::is_socket_inet(
                    fd,
                    None,
                    Some(SocketType::Stream),
                    Listening::IsListening,
                    None,
                )? {
                    // Safety:
                    // - The socket was passed from systemd
                    // - It's confirmed to be a TCP (streaming inet) socket
                    // - It's also confirmed to be a TcpListener (it's in listening mode)
                    let std_socket = unsafe { std::net::TcpListener::from_raw_fd(fd) };
                    Socket::Tcp(tokio::net::TcpListener::from_std(std_socket)?)
                } else if systemd::daemon::is_socket_unix(
                    fd,
                    None,
                    Listening::IsListening,
                    None::<&str>,
                )? {
                    // Safety:
                    // - The socket was passed from systemd
                    // - It's confirmed to be a Unix (streaming unix) socket
                    // - It's also confirmed to be a UnixListener (it's in listening mode)
                    let std_socket = unsafe { std::os::unix::net::UnixListener::from_raw_fd(fd) };
                    Socket::Unix(tokio::net::UnixListener::from_std(std_socket)?)
                } else {
                    todo!("sockets other than inet or unix are not yet supported")
                }
            } else {
                todo!("file descriptors other than sockets are not yet supported")
            };
            Ok(dbg!(socket))
        })
        .collect::<Result<Vec<_>>>()?;

    dbg!(&listeners);

    let Socket::Tcp(listener) = listeners.remove(0) else {
        panic!("unix sockets are not supported yet")
    };

    // if std::env::args().count() != listeners.len() + 1 {
    //     panic!("mismatch of sockets passed in by systemd and ports specified on commandline")
    // }
    // let mut args = std::env::args();
    // let host = args.next().unwrap();

    let folder = format!("/run/user/1000");
    tokio::fs::create_dir_all(&folder).await?;
    let socket = format!("{folder}/sock");

    let mut ssh = Command::new("ssh")
        .args(["-N", &format!("-L{socket}:localhost:12346"), "localhost"])
        .spawn()?;

    let ssh = tokio::spawn(async move { ssh.wait().await });
    tokio::time::sleep(Duration::from_secs(2)).await;

    while let Ok((mut connection, _addr)) = listener.accept().await {
        eprintln!("{:?}", connection);
        let mut ssh_conn = UnixSocket::new_stream()?.connect(&socket).await.unwrap();
        eprintln!("{:?}", ssh_conn);
        tokio::io::copy_bidirectional(&mut connection, &mut ssh_conn)
            .await
            .unwrap();
    }

    ssh.await.unwrap().unwrap();
    panic!("debug");
    // // Ok(())
}

#[derive(Debug)]
enum Socket {
    Tcp(tokio::net::TcpListener),
    Unix(tokio::net::UnixListener),
}
