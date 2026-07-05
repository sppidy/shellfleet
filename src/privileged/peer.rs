#[cfg(unix)]
pub fn peer_uid(stream: &tokio::net::UnixStream) -> Result<u32, String> {
    use std::os::fd::AsRawFd;
    let mut credential = libc::ucred {
        pid: 0,
        uid: 0,
        gid: 0,
    };
    let mut length = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    let result = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            (&mut credential as *mut libc::ucred).cast(),
            &mut length,
        )
    };
    if result == 0 {
        Ok(credential.uid)
    } else {
        Err(std::io::Error::last_os_error().to_string())
    }
}

#[cfg(unix)]
pub fn configured_agent_uid() -> Result<u32, String> {
    if let Ok(raw) = std::env::var("SHELLFLEET_AGENT_UID") {
        return raw
            .parse()
            .map_err(|_| "invalid SHELLFLEET_AGENT_UID".into());
    }
    let passwd = std::fs::read_to_string("/etc/passwd").map_err(|error| error.to_string())?;
    passwd
        .lines()
        .find_map(|line| {
            let mut fields = line.split(':');
            (fields.next()? == "shellfleet")
                .then(|| fields.nth(1)?.parse::<u32>().ok())
                .flatten()
        })
        .ok_or_else(|| "shellfleet service user does not exist".into())
}
