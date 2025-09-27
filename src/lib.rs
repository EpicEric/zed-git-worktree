use std::{
    ffi::OsString,
    path::{Path, PathBuf},
};

use zed_extension_api::{
    self as zed, Result, SlashCommand, SlashCommandArgumentCompletion, SlashCommandOutput,
    SlashCommandOutputSection, Worktree, process::Command,
};

struct GitWorktree {
    worktree: String,
    head: String,
    branch: String,
}

struct GitWorktreeExtension;

impl GitWorktreeExtension {
    fn worktree_path_for_branch(&self, base_path: &str, branch: &str) -> Result<PathBuf> {
        let path_prefix: OsString = base_path.to_owned().into();
        let path_suffix: OsString = branch.replace('/', "-").replace('\\', "-").into();
        Ok(PathBuf::from(path_prefix)
            .parent()
            .ok_or_else(|| "can't get worktree path from root path".to_string())?
            .join(Path::new(&path_suffix)))
    }
}

impl GitWorktreeExtension {
    fn zed_open(&self, path: &Path) -> Result<()> {
        let output = Command::new("zed")
            .args([path.to_string_lossy().as_ref()])
            .output()?;
        match output.status {
            None => Err(format!("`zed {path:?}` was interrupted by a signal")),
            Some(exit_code) if exit_code != 0 => Err(format!(
                "`zed {path:?}` exited with non-zero status code {exit_code}"
            )),
            Some(_) => Ok(()),
        }
    }

    fn git_branch_list(&self) -> Result<Vec<String>> {
        let output = Command::new("git").args(["branch", "-a", "-l"]).output()?;
        match output.status {
            None => return Err("`git branch -l` was interrupted by a signal".into()),
            Some(exit_code) if exit_code != 0 => {
                return Err(format!(
                    "`git branch -l` exited with non-zero status code {exit_code}"
                ));
            }
            Some(_) => (),
        }
        let stdout = String::from_utf8(output.stdout)
            .map_err(|_| "`git branch -l` returned invalid UTF-8".to_string())?;

        Ok(stdout
            .trim()
            .split_whitespace()
            .map(|s| s.to_string())
            .collect())
    }

    fn git_worktree_list(&self) -> Result<Vec<GitWorktree>> {
        let output = Command::new("git")
            .args(["worktree", "list", "--porcelain"])
            .output()?;
        match output.status {
            None => return Err("`git worktree list` was interrupted by a signal".into()),
            Some(exit_code) if exit_code != 0 => {
                return Err(format!(
                    "`git worktree list` exited with non-zero status code {exit_code}"
                ));
            }
            Some(_) => (),
        }
        let stdout = String::from_utf8(output.stdout)
            .map_err(|_| "`git worktree list` returned invalid UTF-8".to_string())?;

        Ok(stdout
            .split("\n\n")
            .filter_map(|worktree_str| {
                let mut worktree = None;
                let mut head = None;
                let mut branch = None;

                for line in worktree_str.split('\n') {
                    if let Some(path) = line.strip_prefix("worktree ") {
                        worktree = Some(path.into());
                    } else if let Some(commit) = line.strip_prefix("HEAD ") {
                        head = Some(commit.into());
                    } else if let Some(git_ref) = line.strip_prefix("branch ") {
                        branch = Some(git_ref.into());
                    }
                }

                match (worktree, head, branch) {
                    (Some(worktree), Some(head), Some(branch)) => Some(GitWorktree {
                        worktree,
                        branch,
                        head,
                    }),
                    (Some(worktree), Some(head), None) => Some(GitWorktree {
                        worktree,
                        branch: head.clone(),
                        head,
                    }),
                    _ => None,
                }
            })
            .collect())
    }

    fn git_worktree_add(&self, base_path: &str, branch: &str) -> Result<PathBuf> {
        let path = self.worktree_path_for_branch(base_path, branch)?;
        let output = Command::new("git")
            .args([
                "worktree",
                "add",
                "-b",
                branch,
                path.to_string_lossy().as_ref(),
            ])
            .output()?;
        match output.status {
            None => Err("`git worktree add` was interrupted by a signal".into()),
            Some(exit_code) if exit_code != 0 => Err(format!(
                "`git worktree add` exited with non-zero status code {exit_code}"
            )),
            Some(_) => Ok(path),
        }
    }

    fn git_worktree_remove(&self, path: &str) -> Result<()> {
        let output = Command::new("git")
            .args(["worktree", "remove", path])
            .output()?;
        match output.status {
            None => Err("`git worktree remove` was interrupted by a signal".into()),
            Some(exit_code) if exit_code != 0 => Err(format!(
                "`git worktree remove` exited with non-zero status code {exit_code}"
            )),
            Some(_) => Ok(()),
        }
    }
}

impl zed::Extension for GitWorktreeExtension {
    fn new() -> Self
    where
        Self: Sized,
    {
        GitWorktreeExtension
    }

    fn complete_slash_command_argument(
        &self,
        command: SlashCommand,
        _args: Vec<String>,
    ) -> Result<Vec<zed_extension_api::SlashCommandArgumentCompletion>> {
        match command.name.as_str() {
            "list" => Ok(vec![]),
            "add" => Ok(self
                .git_branch_list()?
                .into_iter()
                .map(|branch| SlashCommandArgumentCompletion {
                    label: branch.clone(),
                    new_text: branch,
                    run_command: true,
                })
                .collect()),
            "remove" => Ok(self
                .git_worktree_list()?
                .into_iter()
                .map(|worktree| SlashCommandArgumentCompletion {
                    label: worktree.head,
                    new_text: worktree.worktree,
                    run_command: true,
                })
                .collect()),
            command => Err(format!("unknown slash command: \"{command}\"")),
        }
    }

    fn run_slash_command(
        &self,
        command: SlashCommand,
        args: Vec<String>,
        worktree: Option<&Worktree>,
    ) -> Result<SlashCommandOutput> {
        match command.name.as_str() {
            "list" => {
                let worktrees = self.git_worktree_list()?;
                let sections_len = worktrees.len();
                let (text, sections) = worktrees.into_iter().fold(
                    (String::new(), Vec::with_capacity(sections_len)),
                    |mut acc, worktree| {
                        acc.1.push(SlashCommandOutputSection {
                            range: (acc.0.len()..acc.0.len() + worktree.branch.len()).into(),
                            label: worktree.worktree,
                        });
                        (format!("{}{}", acc.0, worktree.branch), acc.1)
                    },
                );
                Ok(SlashCommandOutput { sections, text })
            }
            "add" => {
                let Some(branch) = args.first() else {
                    return Err("no option selected".to_string());
                };
                let Some(worktree) = worktree else {
                    return Err("no worktree found".to_string());
                };
                let path = self.git_worktree_add(worktree.root_path().as_ref(), branch)?;
                self.zed_open(&path)?;

                let text = format!("Worktree `{branch}` added.");

                Ok(SlashCommandOutput {
                    sections: vec![SlashCommandOutputSection {
                        range: (0..text.len()).into(),
                        label: format!("Add"),
                    }],
                    text,
                })
            }
            "remove" => {
                let Some(path) = args.first() else {
                    return Err("no option selected".to_string());
                };
                self.git_worktree_remove(path)?;

                let text = format!("Worktree `{path}` removed.");

                Ok(SlashCommandOutput {
                    sections: vec![SlashCommandOutputSection {
                        range: (0..text.len()).into(),
                        label: format!("Remove"),
                    }],
                    text,
                })
            }
            command => Err(format!("unknown slash command: \"{command}\"")),
        }
    }
}

zed::register_extension!(GitWorktreeExtension);
