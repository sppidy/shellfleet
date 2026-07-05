use shared::trusted::TrustedOperation;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    Two,
}

pub fn classify(operation: &TrustedOperation) -> Result<Tier, String> {
    match operation {
        TrustedOperation::RootCommand {
            program,
            args,
            timeout_secs,
        } => {
            if !program.starts_with('/') || program.contains('\0') || program.len() > 4096 {
                return Err("root command program must be a bounded absolute path".into());
            }
            if args.len() > 256
                || args
                    .iter()
                    .any(|arg| arg.contains('\0') || arg.len() > 16 * 1024)
            {
                return Err("root command arguments exceed policy limits".into());
            }
            if !(1..=3600).contains(timeout_secs) {
                return Err("root command timeout is outside policy limits".into());
            }
        }
        TrustedOperation::RootPty {
            shell,
            ttl_secs,
            cols,
            rows,
        } => {
            if !matches!(shell.as_str(), "/bin/bash" | "/bin/sh" | "/usr/bin/bash") {
                return Err("root PTY shell is not locally allowed".into());
            }
            if !(60..=3600).contains(ttl_secs) || *cols == 0 || *rows == 0 {
                return Err("root PTY limits are invalid".into());
            }
        }
    }
    Ok(Tier::Two)
}

pub fn policy_version() -> String {
    std::env::var("SHELLFLEET_PRIVILEGED_POLICY_VERSION").unwrap_or_else(|_| "builtin-v1".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_supported_root_operation_is_tier_two_and_bounded() {
        assert_eq!(
            classify(&TrustedOperation::RootPty {
                shell: "/bin/bash".into(),
                ttl_secs: 600,
                cols: 80,
                rows: 24,
            }),
            Ok(Tier::Two)
        );
        assert!(
            classify(&TrustedOperation::RootCommand {
                program: "sh".into(),
                args: vec![],
                timeout_secs: 10,
            })
            .is_err()
        );
    }
}
