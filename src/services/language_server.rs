//! Safe discovery boundary for optional language-server executables.
//!
//! This service does not start a child process yet. It resolves only candidates
//! declared by Lumen's built-in language registry; workspace configuration is
//! never interpreted as an executable command.
use lumen_language_services::{LanguageDescriptor, ServerDescriptor};
use std::{env, path::PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ServerAvailability {
    NotSupported,
    Missing { candidates: Vec<String> },
    Available { executable: PathBuf },
}

pub(crate) trait LanguageServerService: Send + Sync {
    fn availability(&self, language: &LanguageDescriptor) -> ServerAvailability;
}

#[derive(Default)]
pub(crate) struct LocalLanguageServer;

impl LanguageServerService for LocalLanguageServer {
    fn availability(&self, language: &LanguageDescriptor) -> ServerAvailability {
        let Some(server) = language.server else {
            return ServerAvailability::NotSupported;
        };
        discover(server, env::var_os("PATH").as_deref())
    }
}

fn discover(server: ServerDescriptor, path: Option<&std::ffi::OsStr>) -> ServerAvailability {
    let candidates = server
        .executable_candidates
        .iter()
        .map(|candidate| (*candidate).to_owned())
        .collect();
    let paths = path.map(env::split_paths).into_iter().flatten();
    for directory in paths {
        for candidate in server.executable_candidates {
            let executable = directory.join(candidate);
            if executable.is_file() {
                return ServerAvailability::Available { executable };
            }
            #[cfg(windows)]
            {
                let executable = directory.join(format!("{candidate}.exe"));
                if executable.is_file() {
                    return ServerAvailability::Available { executable };
                }
            }
        }
    }
    ServerAvailability::Missing { candidates }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lumen_language_services::LanguageRegistry;

    #[test]
    fn discovery_finds_only_registered_candidates() {
        let temporary = tempfile::tempdir().unwrap();
        let analyzer = temporary.path().join("rust-analyzer");
        std::fs::write(&analyzer, []).unwrap();
        let rust = LanguageRegistry::by_id(lumen_language_services::LanguageId::RUST).unwrap();
        let path = env::join_paths([temporary.path()]).unwrap();
        assert_eq!(
            discover(rust.server.unwrap(), Some(&path)),
            ServerAvailability::Available {
                executable: analyzer
            }
        );
    }

    #[test]
    fn discovery_reports_missing_and_unsupported_languages_without_execution() {
        let go = LanguageRegistry::by_id(lumen_language_services::LanguageId::GO).unwrap();
        assert_eq!(
            discover(go.server.unwrap(), None),
            ServerAvailability::Missing {
                candidates: vec!["gopls".into()]
            }
        );
        let json = LanguageRegistry::by_id(lumen_language_services::LanguageId::JSON).unwrap();
        assert_eq!(
            LocalLanguageServer.availability(json),
            ServerAvailability::NotSupported
        );
    }
}
