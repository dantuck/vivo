use std::str::FromStr;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum Step {
    #[default]
    Backup,
    Check,
    Forget,
    Sync,
}

impl FromStr for Step {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "backup" => Ok(Step::Backup),
            "check" => Ok(Step::Check),
            "forget" => Ok(Step::Forget),
            "sync" => Ok(Step::Sync),
            _ => Err(format!(
                "unknown step '{s}'. valid values: backup, check, forget, sync"
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_all_steps() {
        assert_eq!(Step::from_str("backup").unwrap(), Step::Backup);
        assert_eq!(Step::from_str("check").unwrap(), Step::Check);
        assert_eq!(Step::from_str("forget").unwrap(), Step::Forget);
        assert_eq!(Step::from_str("sync").unwrap(), Step::Sync);
    }

    #[test]
    fn parses_case_insensitive() {
        assert_eq!(Step::from_str("CHECK").unwrap(), Step::Check);
        assert_eq!(Step::from_str("Forget").unwrap(), Step::Forget);
    }

    #[test]
    fn ordering() {
        assert!(Step::Backup < Step::Check);
        assert!(Step::Check < Step::Forget);
        assert!(Step::Forget < Step::Sync);
    }

    #[test]
    fn unknown_step_errors() {
        let err = Step::from_str("invalid").unwrap_err();
        assert!(err.contains("backup, check, forget, sync"));
    }

    #[test]
    fn default_is_backup() {
        assert_eq!(Step::default(), Step::Backup);
    }
}
