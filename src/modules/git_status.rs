use once_cell::sync::OnceCell;
use regex::Regex;

use super::{Context, Module, RootModuleConfig};

use crate::configs::git_status::GitStatusConfig;
use crate::formatter::StringFormatter;
use crate::segment::Segment;
use std::ffi::OsStr;
use std::sync::Arc;

const ALL_STATUS_FORMAT: &str = "$conflicted$stashed$work_dir$staged$untracked";

/// Creates a module with the Git branch in the current directory
///
/// Will display the branch name if the current directory is a git repo
/// By default, the following symbols will be used to represent the repo's status:
///   - `=` – This branch has merge conflicts
///   - `⇡` – This branch is ahead of the branch being tracked
///   - `⇣` – This branch is behind of the branch being tracked
///   - `⇕` – This branch has diverged from the branch being tracked
///   - `` – This branch is up-to-date with the branch being tracked
///   - `?` — There are untracked files in the working directory
///   - `$` — A stash exists for the local repository
///   - `!` — There are file modifications in the working directory
///   - `+` — There are file modifications in the staging area
pub fn module<'a>(context: &'a Context) -> Option<Module<'a>> {
    let info = Arc::new(GitStatusInfo::load(context));

    let mut module = context.new_module("git_status");
    let config: GitStatusConfig = GitStatusConfig::try_load(module.config);

    //Return None if not in git repository
    context.get_repo().ok()?;

    let parsed = StringFormatter::new(config.format).and_then(|formatter| {
        formatter
            .map_meta(|variable, _| match variable {
                "all_status" => Some(ALL_STATUS_FORMAT),
                _ => None,
            })
            .map_style(|variable: &str| match variable {
                "style" => Some(Ok(config.style)),
                _ => None,
            })
            .map_variables_to_segments(|variable: &str| {
                let info = Arc::clone(&info);
                let segments = match variable {
                    "stashed" => info.get_stashed().and_then(|count| {
                        format_count(config.stashed, "git_status.stashed", context, count)
                    }),
                    "ahead_behind" => info.get_ahead_behind().and_then(|(ahead, behind)| {
                        let (ahead, behind) = (ahead?, behind?);
                        if ahead > 0 && behind > 0 {
                            format_text(
                                config.diverged,
                                "git_status.diverged",
                                context,
                                |variable| match variable {
                                    "ahead_count" => Some(ahead.to_string()),
                                    "behind_count" => Some(behind.to_string()),
                                    _ => None,
                                },
                            )
                        } else if ahead > 0 && behind == 0 {
                            format_count(config.ahead, "git_status.ahead", context, ahead)
                        } else if behind > 0 && ahead == 0 {
                            format_count(config.behind, "git_status.behind", context, behind)
                        } else {
                            format_symbol(config.up_to_date, "git_status.up_to_date", context)
                        }
                    }),
                    "conflicted" => info.get_conflicted().and_then(|count| {
                        format_count(config.conflicted, "git_status.conflicted", context, count)
                    }),
                    "work_dir" => info.get_work_dir().and_then(|work_dir_status| {
                        let total_count = work_dir_status.total();
                        format_multivar_count(
                            config.work_dir,
                            "git_status.work_dir",
                            context,
                            |variable| match variable {
                                "added_count" => Some(work_dir_status.added.to_string()),
                                "count" => Some(total_count.to_string()),
                                "deleted_count" => Some(work_dir_status.deleted.to_string()),
                                "modified_count" => Some(work_dir_status.modified.to_string()),
                                _ => None,
                            },
                            total_count,
                        )
                    }),
                    "staged" => info.get_staged().and_then(|staging_status| {
                        let total_count = staging_status.total();
                        format_multivar_count(
                            config.staged,
                            "git_status.staged",
                            context,
                            |variable| match variable {
                                "added_count" => Some(staging_status.added.to_string()),
                                "count" => Some(total_count.to_string()),
                                "deleted_count" => Some(staging_status.deleted.to_string()),
                                "modified_count" => Some(staging_status.modified.to_string()),
                                "renamed_count" => Some(staging_status.renamed.to_string()),
                                _ => None,
                            },
                            total_count,
                        )
                    }),
                    "untracked" => info.get_untracked().and_then(|count| {
                        format_count(config.untracked, "git_status.untracked", context, count)
                    }),
                    _ => None,
                };
                segments.map(Ok)
            })
            .parse(None, Some(context))
    });

    module.set_segments(match parsed {
        Ok(segments) => {
            if segments.is_empty() {
                return None;
            } else {
                segments
            }
        }
        Err(error) => {
            log::warn!("Error in module `git_status`:\n{}", error);
            return None;
        }
    });

    Some(module)
}

struct GitStatusInfo<'a> {
    context: &'a Context<'a>,
    repo_status: OnceCell<Option<RepoStatus>>,
    stashed_count: OnceCell<Option<usize>>,
}

impl<'a> GitStatusInfo<'a> {
    pub fn load(context: &'a Context) -> Self {
        Self {
            context,
            repo_status: OnceCell::new(),
            stashed_count: OnceCell::new(),
        }
    }

    pub fn get_ahead_behind(&self) -> Option<(Option<usize>, Option<usize>)> {
        self.get_repo_status().map(|data| (data.ahead, data.behind))
    }

    pub fn get_repo_status(&self) -> &Option<RepoStatus> {
        self.repo_status
            .get_or_init(|| match get_repo_status(self.context) {
                Some(repo_status) => Some(repo_status),
                None => {
                    log::debug!("get_repo_status: git status execution failed");
                    None
                }
            })
    }

    pub fn get_stashed(&self) -> &Option<usize> {
        self.stashed_count
            .get_or_init(|| match get_stashed_count(self.context) {
                Some(stashed_count) => Some(stashed_count),
                None => {
                    log::debug!("get_stashed_count: git stash execution failed");
                    None
                }
            })
    }

    pub fn get_conflicted(&self) -> Option<usize> {
        self.get_repo_status().map(|data| data.conflicted)
    }

    pub fn get_work_dir(&self) -> Option<WorkDirStatus> {
        self.get_repo_status().map(|data| data.work_dir)
    }

    pub fn get_staged(&self) -> Option<StagingStatus> {
        self.get_repo_status().map(|data| data.staged)
    }

    pub fn get_untracked(&self) -> Option<usize> {
        self.get_repo_status().map(|data| data.untracked)
    }
}

/// Gets the number of files in various git states (staged, modified, deleted, etc...)
fn get_repo_status(context: &Context) -> Option<RepoStatus> {
    log::debug!("New repo status created");

    let mut repo_status = RepoStatus::default();
    let status_output = context.exec_cmd(
        "git",
        &[
            OsStr::new("-C"),
            context.current_dir.as_os_str(),
            OsStr::new("--no-optional-locks"),
            OsStr::new("status"),
            OsStr::new("--porcelain=2"),
            OsStr::new("--branch"),
        ],
    )?;
    let statuses = status_output.stdout.lines();

    statuses.for_each(|status| {
        if status.starts_with("# branch.ab ") {
            repo_status.set_ahead_behind(status);
        } else if !status.starts_with('#') {
            repo_status.add(status);
        }
    });

    Some(repo_status)
}

fn get_stashed_count(context: &Context) -> Option<usize> {
    let stash_output = context.exec_cmd(
        "git",
        &[
            OsStr::new("-C"),
            context.current_dir.as_os_str(),
            OsStr::new("--no-optional-locks"),
            OsStr::new("stash"),
            OsStr::new("list"),
        ],
    )?;

    Some(stash_output.stdout.trim().lines().count())
}

#[derive(Default, Debug, Copy, Clone)]
struct WorkDirStatus {
    added: usize,
    deleted: usize,
    modified: usize,
}

impl WorkDirStatus {
    fn total(self) -> usize {
        self.added + self.deleted + self.modified
    }
}

#[derive(Default, Debug, Copy, Clone)]
struct StagingStatus {
    added: usize,
    deleted: usize,
    modified: usize,
    renamed: usize,
}

impl StagingStatus {
    fn total(self) -> usize {
        self.added + self.deleted + self.modified + self.renamed
    }
}

#[derive(Default, Debug, Copy, Clone)]
struct RepoStatus {
    ahead: Option<usize>,
    behind: Option<usize>,
    conflicted: usize,
    work_dir: WorkDirStatus,
    staged: StagingStatus,
    untracked: usize,
}

impl RepoStatus {
    fn is_staging_deleted(short_status: &str) -> bool {
        short_status.starts_with('D')
    }

    fn is_work_dir_deleted(short_status: &str) -> bool {
        short_status.ends_with('D')
    }

    fn is_staging_modified(short_status: &str) -> bool {
        short_status.starts_with('M')
    }

    fn is_work_dir_modified(short_status: &str) -> bool {
        short_status.ends_with('M')
    }

    fn is_staging_added(short_status: &str) -> bool {
        short_status.starts_with('A')
    }

    fn is_workdir_added(short_status: &str) -> bool {
        short_status.ends_with('A')
    }

    fn parse_normal_status(&mut self, short_status: &str) {
        if Self::is_staging_deleted(short_status) {
            self.staged.deleted += 1
        }
        if Self::is_work_dir_deleted(short_status) {
            self.work_dir.deleted += 1
        }
        if Self::is_staging_modified(short_status) {
            self.staged.modified += 1
        }
        if Self::is_work_dir_modified(short_status) {
            self.work_dir.modified += 1
        }
        if Self::is_staging_added(short_status) {
            self.staged.added += 1
        }
        if Self::is_workdir_added(short_status) {
            self.work_dir.added += 1
        }
    }

    fn add(&mut self, s: &str) {
        match s.chars().next() {
            Some('1') => self.parse_normal_status(&s[2..4]),
            Some('2') => {
                self.staged.renamed += 1;
                self.parse_normal_status(&s[2..4])
            }
            Some('u') => self.conflicted += 1,
            Some('?') => self.untracked += 1,
            Some('!') => (),
            Some(_) => log::error!("Unknown line type in git status output"),
            None => log::error!("Missing line type in git status output"),
        }
    }

    fn set_ahead_behind(&mut self, s: &str) {
        let re = Regex::new(r"branch\.ab \+([0-9]+) \-([0-9]+)").unwrap();

        if let Some(caps) = re.captures(s) {
            self.ahead = caps.get(1).unwrap().as_str().parse::<usize>().ok();
            self.behind = caps.get(2).unwrap().as_str().parse::<usize>().ok();
        }
    }
}

fn format_text<F>(
    format_str: &str,
    config_path: &str,
    context: &Context,
    mapper: F,
) -> Option<Vec<Segment>>
where
    F: Fn(&str) -> Option<String> + Send + Sync,
{
    if let Ok(formatter) = StringFormatter::new(format_str) {
        formatter
            .map(|variable| mapper(variable).map(Ok))
            .parse(None, Some(context))
            .ok()
    } else {
        log::warn!("Error parsing format string `{}`", &config_path);
        None
    }
}

fn format_multivar_count<F>(
    format_str: &str,
    config_path: &str,
    context: &Context,
    mapper: F,
    total_count: usize,
) -> Option<Vec<Segment>>
where
    F: Fn(&str) -> Option<String> + Send + Sync,
{
    if total_count == 0 {
        return None;
    }

    format_text(format_str, config_path, context, mapper)
}

fn format_count(
    format_str: &str,
    config_path: &str,
    context: &Context,
    count: usize,
) -> Option<Vec<Segment>> {
    if count == 0 {
        return None;
    }

    format_text(
        format_str,
        config_path,
        context,
        |variable| match variable {
            "count" => Some(count.to_string()),
            _ => None,
        },
    )
}

fn format_symbol(format_str: &str, config_path: &str, context: &Context) -> Option<Vec<Segment>> {
    format_text(format_str, config_path, context, |_variable| None)
}

#[cfg(test)]
mod tests {
    use ansi_term::{ANSIStrings, Color};
    use std::ffi::OsStr;
    use std::fs::{self, File};
    use std::io::{self, prelude::*};
    use std::path::Path;

    use crate::test::{fixture_repo, FixtureProvider, ModuleRenderer};
    use crate::utils::create_command;

    /// Right after the calls to git the filesystem state may not have finished
    /// updating yet causing some of the tests to fail. These barriers are placed
    /// after each call to git.
    /// This barrier is windows-specific though other operating systems may need it
    /// in the future.
    #[cfg(not(windows))]
    fn barrier() {}
    #[cfg(windows)]
    fn barrier() {
        std::thread::sleep(std::time::Duration::from_millis(500));
    }

    #[allow(clippy::unnecessary_wraps)]
    fn format_output(symbols: &str) -> Option<String> {
        Some(format!(
            "{} ",
            Color::Red.bold().paint(format!("[{}]", symbols))
        ))
    }

    #[test]
    fn show_nothing_on_empty_dir() -> io::Result<()> {
        let repo_dir = tempfile::tempdir()?;

        let actual = ModuleRenderer::new("git_status")
            .path(repo_dir.path())
            .collect();
        let expected = None;

        assert_eq!(expected, actual);
        repo_dir.close()
    }

    #[test]
    fn shows_behind() -> io::Result<()> {
        let repo_dir = fixture_repo(FixtureProvider::Git)?;

        behind(repo_dir.path())?;

        let actual = ModuleRenderer::new("git_status")
            .path(repo_dir.path())
            .collect();
        let expected = format_output("⇣");

        assert_eq!(expected, actual);
        repo_dir.close()
    }

    #[test]
    fn shows_behind_with_count() -> io::Result<()> {
        let repo_dir = fixture_repo(FixtureProvider::Git)?;

        behind(repo_dir.path())?;

        let actual = ModuleRenderer::new("git_status")
            .config(toml::toml! {
                [git_status]
                behind = "⇣$count"
            })
            .path(repo_dir.path())
            .collect();
        let expected = format_output("⇣1");

        assert_eq!(expected, actual);
        repo_dir.close()
    }

    #[test]
    fn shows_ahead() -> io::Result<()> {
        let repo_dir = fixture_repo(FixtureProvider::Git)?;

        File::create(repo_dir.path().join("readme.md"))?.sync_all()?;
        ahead(repo_dir.path())?;

        let actual = ModuleRenderer::new("git_status")
            .path(&repo_dir.path())
            .collect();
        let expected = format_output("⇡");

        assert_eq!(expected, actual);
        repo_dir.close()
    }

    #[test]
    fn shows_ahead_with_count() -> io::Result<()> {
        let repo_dir = fixture_repo(FixtureProvider::Git)?;

        File::create(repo_dir.path().join("readme.md"))?.sync_all()?;
        ahead(repo_dir.path())?;

        let actual = ModuleRenderer::new("git_status")
            .config(toml::toml! {
                [git_status]
                ahead="⇡$count"
            })
            .path(&repo_dir.path())
            .collect();
        let expected = format_output("⇡1");

        assert_eq!(expected, actual);
        repo_dir.close()
    }

    #[test]
    fn shows_diverged() -> io::Result<()> {
        let repo_dir = fixture_repo(FixtureProvider::Git)?;

        diverge(repo_dir.path())?;

        let actual = ModuleRenderer::new("git_status")
            .path(&repo_dir.path())
            .collect();
        let expected = format_output("⇕");

        assert_eq!(expected, actual);
        repo_dir.close()
    }

    #[test]
    fn shows_diverged_with_count() -> io::Result<()> {
        let repo_dir = fixture_repo(FixtureProvider::Git)?;

        diverge(repo_dir.path())?;

        let actual = ModuleRenderer::new("git_status")
            .config(toml::toml! {
                [git_status]
                diverged=r"⇕⇡$ahead_count⇣$behind_count"
            })
            .path(&repo_dir.path())
            .collect();
        let expected = format_output("⇕⇡1⇣1");

        assert_eq!(expected, actual);
        repo_dir.close()
    }

    #[test]
    fn shows_up_to_date_with_upstream() -> io::Result<()> {
        let repo_dir = fixture_repo(FixtureProvider::Git)?;

        let actual = ModuleRenderer::new("git_status")
            .config(toml::toml! {
                [git_status]
                up_to_date="✓"
            })
            .path(&repo_dir.path())
            .collect();
        let expected = format_output("✓");

        assert_eq!(expected, actual);
        repo_dir.close()
    }

    #[test]
    fn shows_conflicted() -> io::Result<()> {
        let repo_dir = fixture_repo(FixtureProvider::Git)?;

        create_conflict(repo_dir.path())?;

        let actual = ModuleRenderer::new("git_status")
            .path(&repo_dir.path())
            .collect();
        let expected = format_output("=");

        assert_eq!(expected, actual);
        repo_dir.close()
    }

    #[test]
    fn shows_conflicted_with_count() -> io::Result<()> {
        let repo_dir = fixture_repo(FixtureProvider::Git)?;

        create_conflict(repo_dir.path())?;

        let actual = ModuleRenderer::new("git_status")
            .config(toml::toml! {
                [git_status]
                conflicted = "=$count"
            })
            .path(&repo_dir.path())
            .collect();
        let expected = format_output("=1");

        assert_eq!(expected, actual);
        repo_dir.close()
    }

    #[test]
    fn shows_untracked_file() -> io::Result<()> {
        let repo_dir = fixture_repo(FixtureProvider::Git)?;

        create_untracked(repo_dir.path())?;

        let actual = ModuleRenderer::new("git_status")
            .path(&repo_dir.path())
            .collect();
        let expected = format_output("?");

        assert_eq!(expected, actual);
        repo_dir.close()
    }

    #[test]
    fn shows_untracked_file_with_count() -> io::Result<()> {
        let repo_dir = fixture_repo(FixtureProvider::Git)?;

        create_untracked(repo_dir.path())?;

        let actual = ModuleRenderer::new("git_status")
            .config(toml::toml! {
                [git_status]
                untracked = "?$count"
            })
            .path(&repo_dir.path())
            .collect();
        let expected = format_output("?1");

        assert_eq!(expected, actual);
        repo_dir.close()
    }

    #[test]
    fn doesnt_show_untracked_file_if_disabled() -> io::Result<()> {
        let repo_dir = fixture_repo(FixtureProvider::Git)?;

        create_untracked(repo_dir.path())?;

        create_command("git")?
            .args(&["config", "status.showUntrackedFiles", "no"])
            .current_dir(repo_dir.path())
            .output()?;
        barrier();

        let actual = ModuleRenderer::new("git_status")
            .path(&repo_dir.path())
            .collect();
        let expected = None;

        assert_eq!(expected, actual);
        repo_dir.close()
    }

    #[test]
    fn shows_stashed() -> io::Result<()> {
        let repo_dir = fixture_repo(FixtureProvider::Git)?;
        barrier();

        create_stash(repo_dir.path())?;

        create_command("git")?
            .args(&["reset", "--hard", "HEAD"])
            .current_dir(repo_dir.path())
            .output()?;
        barrier();

        let actual = ModuleRenderer::new("git_status")
            .path(&repo_dir.path())
            .collect();
        let expected = format_output("$");

        assert_eq!(expected, actual);
        repo_dir.close()
    }

    #[test]
    fn shows_stashed_with_count() -> io::Result<()> {
        let repo_dir = fixture_repo(FixtureProvider::Git)?;
        barrier();

        create_stash(repo_dir.path())?;
        barrier();

        create_command("git")?
            .args(&["reset", "--hard", "HEAD"])
            .current_dir(repo_dir.path())
            .output()?;
        barrier();

        let actual = ModuleRenderer::new("git_status")
            .config(toml::toml! {
                [git_status]
                stashed = r"\$$count"
            })
            .path(&repo_dir.path())
            .collect();
        let expected = format_output("$1");

        assert_eq!(expected, actual);
        repo_dir.close()
    }

    #[test]
    fn shows_work_dir_modified_count() -> io::Result<()> {
        let repo_dir = fixture_repo(FixtureProvider::Git)?;

        create_work_dir_modified(repo_dir.path())?;

        let actual = ModuleRenderer::new("git_status")
            .config(toml::toml! {
                [git_status]
                work_dir = "!$modified_count"
            })
            .path(&repo_dir.path())
            .collect();
        let expected = format_output("!1");

        assert_eq!(expected, actual);
        repo_dir.close()
    }

    #[test]
    fn shows_staged_modified_count() -> io::Result<()> {
        let repo_dir = fixture_repo(FixtureProvider::Git)?;

        create_staged_modified(repo_dir.path())?;

        let actual = ModuleRenderer::new("git_status")
            .config(toml::toml! {
                [git_status]
                staged = "+$modified_count"
            })
            .path(&repo_dir.path())
            .collect();
        let expected = format_output("+1");

        assert_eq!(expected, actual);
        repo_dir.close()
    }

    #[test]
    fn shows_added() -> io::Result<()> {
        let repo_dir = fixture_repo(FixtureProvider::Git)?;

        create_added(repo_dir.path())?;

        let actual = ModuleRenderer::new("git_status")
            .path(&repo_dir.path())
            .collect();
        let expected = format_output("!");

        assert_eq!(expected, actual);
        repo_dir.close()
    }

    #[test]
    fn shows_staged_file() -> io::Result<()> {
        let repo_dir = fixture_repo(FixtureProvider::Git)?;

        create_staged(repo_dir.path())?;

        let actual = ModuleRenderer::new("git_status")
            .path(&repo_dir.path())
            .collect();
        let expected = format_output("+");

        assert_eq!(expected, actual);
        repo_dir.close()
    }

    #[test]
    fn shows_staged_file_with_count() -> io::Result<()> {
        let repo_dir = fixture_repo(FixtureProvider::Git)?;

        create_staged(repo_dir.path())?;

        let actual = ModuleRenderer::new("git_status")
            .config(toml::toml! {
                [git_status]
                staged = "+[$count](green)"
            })
            .path(&repo_dir.path())
            .collect();
        let expected = Some(format!(
            "{} ",
            ANSIStrings(&[
                Color::Red.bold().paint("[+"),
                Color::Green.paint("1"),
                Color::Red.bold().paint("]"),
            ])
        ));

        assert_eq!(expected, actual);
        repo_dir.close()
    }

    #[test]
    fn shows_staged_and_modified_file() -> io::Result<()> {
        let repo_dir = fixture_repo(FixtureProvider::Git)?;

        create_staged_and_modified(repo_dir.path())?;

        let actual = ModuleRenderer::new("git_status")
            .path(&repo_dir.path())
            .collect();
        let expected = format_output("!+");

        assert_eq!(expected, actual);
        repo_dir.close()
    }

    #[test]
    fn shows_renamed_file() -> io::Result<()> {
        let repo_dir = fixture_repo(FixtureProvider::Git)?;

        create_renamed(repo_dir.path())?;

        let actual = ModuleRenderer::new("git_status")
            .path(&repo_dir.path())
            .collect();
        let expected = format_output("+");

        assert_eq!(expected, actual);
        repo_dir.close()
    }

    #[test]
    fn shows_renamed_file_with_count() -> io::Result<()> {
        let repo_dir = fixture_repo(FixtureProvider::Git)?;

        create_renamed(repo_dir.path())?;

        let actual = ModuleRenderer::new("git_status")
            .config(toml::toml! {
                [git_status]
                staged = "»$renamed_count"
            })
            .path(&repo_dir.path())
            .collect();
        let expected = format_output("»1");

        assert_eq!(expected, actual);
        repo_dir.close()
    }

    #[test]
    fn shows_renamed_and_modified_file() -> io::Result<()> {
        let repo_dir = fixture_repo(FixtureProvider::Git)?;

        create_renamed_and_modified(repo_dir.path())?;

        let actual = ModuleRenderer::new("git_status")
            .path(&repo_dir.path())
            .collect();
        let expected = format_output("!+");

        assert_eq!(expected, actual);
        repo_dir.close()
    }

    #[test]
    fn shows_work_dir_deleted_count() -> io::Result<()> {
        let repo_dir = fixture_repo(FixtureProvider::Git)?;

        create_work_dir_deleted(repo_dir.path())?;

        let actual = ModuleRenderer::new("git_status")
            .config(toml::toml! {
                [git_status]
                work_dir = "!$deleted_count"
            })
            .path(&repo_dir.path())
            .collect();
        let expected = format_output("!1");

        assert_eq!(expected, actual);
        repo_dir.close()
    }

    #[test]
    fn shows_staged_deleted_count() -> io::Result<()> {
        let repo_dir = fixture_repo(FixtureProvider::Git)?;

        create_staged_deleted(repo_dir.path())?;

        let actual = ModuleRenderer::new("git_status")
            .config(toml::toml! {
                [git_status]
                staged = "+$deleted_count"
            })
            .path(&repo_dir.path())
            .collect();
        let expected = format_output("+1");

        assert_eq!(expected, actual);
        repo_dir.close()
    }

    #[test]
    fn doesnt_show_ignored_file() -> io::Result<()> {
        let repo_dir = fixture_repo(FixtureProvider::Git)?;

        create_staged_and_ignored(repo_dir.path())?;

        let actual = ModuleRenderer::new("git_status")
            .path(&repo_dir.path())
            .collect();
        let expected = format_output("+");

        assert_eq!(expected, actual);
        repo_dir.close()
    }

    #[test]
    fn worktree_in_different_dir() -> io::Result<()> {
        let worktree_dir = tempfile::tempdir()?;
        let repo_dir = fixture_repo(FixtureProvider::Git)?;

        create_command("git")?
            .args(&[
                OsStr::new("config"),
                OsStr::new("core.worktree"),
                worktree_dir.path().as_os_str(),
            ])
            .current_dir(repo_dir.path())
            .output()?;

        File::create(worktree_dir.path().join("test_file"))?.sync_all()?;

        let actual = ModuleRenderer::new("git_status")
            .path(&repo_dir.path())
            .collect();
        let expected = format_output("!?");

        assert_eq!(expected, actual);
        worktree_dir.close()?;
        repo_dir.close()
    }

    // Whenever a file is manually renamed, git itself ('git status') does not treat such file as renamed,
    // but as untracked instead. The following test checks if manually deleted and manually renamed
    // files are tracked by git_status module in the same way 'git status' does.
    #[test]
    #[ignore]
    fn ignore_manually_renamed() -> io::Result<()> {
        let repo_dir = fixture_repo(FixtureProvider::Git)?;
        File::create(repo_dir.path().join("a"))?.sync_all()?;
        File::create(repo_dir.path().join("b"))?.sync_all()?;
        create_command("git")?
            .args(&["add", "--all"])
            .current_dir(&repo_dir.path())
            .output()?;
        create_command("git")?
            .args(&["commit", "-m", "add new files", "--no-gpg-sign"])
            .current_dir(&repo_dir.path())
            .output()?;

        fs::remove_file(repo_dir.path().join("a"))?;
        fs::rename(repo_dir.path().join("b"), repo_dir.path().join("c"))?;
        barrier();

        let actual = ModuleRenderer::new("git_status")
            .path(&repo_dir.path())
            .config(toml::toml! {
                [git_status]
                ahead = "A"
                deleted = "D"
                untracked = "U"
                renamed = "R"
            })
            .collect();
        let expected = format_output("DUA");

        assert_eq!(actual, expected);

        repo_dir.close()
    }

    fn ahead(repo_dir: &Path) -> io::Result<()> {
        File::create(repo_dir.join("readme.md"))?.sync_all()?;

        create_command("git")?
            .args(&["commit", "-am", "Update readme", "--no-gpg-sign"])
            .current_dir(&repo_dir)
            .output()?;
        barrier();

        Ok(())
    }

    fn behind(repo_dir: &Path) -> io::Result<()> {
        create_command("git")?
            .args(&["reset", "--hard", "HEAD^"])
            .current_dir(repo_dir)
            .output()?;
        barrier();

        Ok(())
    }

    fn diverge(repo_dir: &Path) -> io::Result<()> {
        create_command("git")?
            .args(&["reset", "--hard", "HEAD^"])
            .current_dir(repo_dir)
            .output()?;
        barrier();

        fs::write(repo_dir.join("Cargo.toml"), " ")?;

        create_command("git")?
            .args(&["commit", "-am", "Update readme", "--no-gpg-sign"])
            .current_dir(repo_dir)
            .output()?;
        barrier();

        Ok(())
    }

    fn create_conflict(repo_dir: &Path) -> io::Result<()> {
        create_command("git")?
            .args(&["reset", "--hard", "HEAD^"])
            .current_dir(repo_dir)
            .output()?;
        barrier();

        fs::write(repo_dir.join("readme.md"), "# goodbye")?;

        create_command("git")?
            .args(&["add", "."])
            .current_dir(repo_dir)
            .output()?;
        barrier();

        create_command("git")?
            .args(&["commit", "-m", "Change readme", "--no-gpg-sign"])
            .current_dir(repo_dir)
            .output()?;
        barrier();

        create_command("git")?
            .args(&["pull", "--rebase"])
            .current_dir(repo_dir)
            .output()?;
        barrier();

        Ok(())
    }

    fn create_stash(repo_dir: &Path) -> io::Result<()> {
        File::create(repo_dir.join("readme.md"))?.sync_all()?;
        barrier();

        create_command("git")?
            .args(&["stash", "--all"])
            .current_dir(repo_dir)
            .output()?;
        barrier();

        Ok(())
    }

    fn create_untracked(repo_dir: &Path) -> io::Result<()> {
        File::create(repo_dir.join("license"))?.sync_all()?;

        Ok(())
    }

    fn create_added(repo_dir: &Path) -> io::Result<()> {
        File::create(repo_dir.join("license"))?.sync_all()?;

        create_command("git")?
            .args(&["add", "-A", "-N"])
            .current_dir(repo_dir)
            .output()?;
        barrier();

        Ok(())
    }

    fn create_work_dir_modified(repo_dir: &Path) -> io::Result<()> {
        File::create(repo_dir.join("readme.md"))?.sync_all()?;

        Ok(())
    }

    fn create_staged_modified(repo_dir: &Path) -> io::Result<()> {
        File::create(repo_dir.join("readme.md"))?.sync_all()?;

        create_command("git")?
            .args(&["add", "-A"])
            .current_dir(repo_dir)
            .output()?;
        barrier();

        Ok(())
    }

    fn create_staged(repo_dir: &Path) -> io::Result<()> {
        File::create(repo_dir.join("license"))?.sync_all()?;

        create_command("git")?
            .args(&["add", "."])
            .current_dir(repo_dir)
            .output()?;
        barrier();

        Ok(())
    }

    fn create_staged_and_modified(repo_dir: &Path) -> io::Result<()> {
        let mut file = File::create(repo_dir.join("readme.md"))?;
        file.sync_all()?;

        create_command("git")?
            .args(&["add", "."])
            .current_dir(repo_dir)
            .output()?;
        barrier();

        writeln!(&mut file, "modified")?;
        file.sync_all()?;

        Ok(())
    }

    fn create_renamed(repo_dir: &Path) -> io::Result<()> {
        create_command("git")?
            .args(&["mv", "readme.md", "readme.md.bak"])
            .current_dir(repo_dir)
            .output()?;
        barrier();

        create_command("git")?
            .args(&["add", "-A"])
            .current_dir(repo_dir)
            .output()?;
        barrier();

        Ok(())
    }

    fn create_renamed_and_modified(repo_dir: &Path) -> io::Result<()> {
        create_command("git")?
            .args(&["mv", "readme.md", "readme.md.bak"])
            .current_dir(repo_dir)
            .output()?;
        barrier();

        create_command("git")?
            .args(&["add", "-A"])
            .current_dir(repo_dir)
            .output()?;
        barrier();

        let mut file = File::create(repo_dir.join("readme.md.bak"))?;
        writeln!(&mut file, "modified")?;
        file.sync_all()?;

        Ok(())
    }

    fn create_staged_deleted(repo_dir: &Path) -> io::Result<()> {
        fs::remove_file(repo_dir.join("readme.md"))?;

        create_command("git")?
            .args(&["add", "-A"])
            .current_dir(repo_dir)
            .output()?;
        barrier();

        Ok(())
    }

    fn create_work_dir_deleted(repo_dir: &Path) -> io::Result<()> {
        fs::remove_file(repo_dir.join("readme.md"))?;

        Ok(())
    }

    fn create_staged_and_ignored(repo_dir: &Path) -> io::Result<()> {
        let mut file = File::create(repo_dir.join(".gitignore"))?;
        writeln!(&mut file, "ignored.txt")?;
        file.sync_all()?;

        create_command("git")?
            .args(&["add", ".gitignore"])
            .current_dir(repo_dir)
            .output()?;
        barrier();

        let mut file = File::create(repo_dir.join("ignored.txt"))?;
        writeln!(&mut file, "modified")?;
        file.sync_all()?;

        Ok(())
    }
}
