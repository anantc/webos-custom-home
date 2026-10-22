use crate::common::{fault, Result};
use std::{
    fs,
    io::Write,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};

pub const LICENSES: &str = include_str!(concat!(env!("OUT_DIR"), "/licenses.txt"));
const PATH_SETUP: &str = "case \":$PATH:\" in\n  *\":$HOME/.local/bin:\"*) ;;\n  *) export PATH=\"$HOME/.local/bin:$PATH\" ;;\nesac\n";

fn shell_files(root: &Path, shell: &Path, zdotdir: Option<&Path>, macos: bool) -> Vec<PathBuf> {
    match shell.file_name().and_then(|s| s.to_str()) {
        Some("zsh") => vec![zdotdir.unwrap_or(root).join(".zshrc")],
        Some("bash") => {
            let mut files = vec![root.join(".bashrc")];
            if macos {
                // Bash login shells read only the first existing file in this order.
                let login = [".bash_profile", ".bash_login", ".profile"]
                    .into_iter()
                    .map(|name| root.join(name))
                    .find(|file| file.exists())
                    .unwrap_or_else(|| root.join(".bash_profile"));
                files.push(login);
            }
            files
        }
        _ => Vec::new(),
    }
}

pub(crate) fn write_atomic(path: &Path, bytes: &[u8], mode: u32) -> Result<()> {
    let parent = path.parent().ok_or(fault("Invalid installation path."))?;
    fs::create_dir_all(parent).map_err(|_| fault("Cannot create installation directory."))?;
    let mut temp = tempfile::NamedTempFile::new_in(parent)
        .map_err(|_| fault("Cannot stage installation file."))?;
    temp.write_all(bytes)
        .map_err(|_| fault("Cannot write installation file."))?;
    temp.as_file()
        .set_permissions(fs::Permissions::from_mode(mode))
        .map_err(|_| fault("Cannot set installation permissions."))?;
    temp.as_file()
        .sync_all()
        .map_err(|_| fault("Cannot flush installation file."))?;
    temp.persist(path)
        .map_err(|_| fault("Cannot replace installation file."))?;
    Ok(())
}

fn prepare_shell(path: &Path) -> Result<(PathBuf, Vec<u8>, u32)> {
    // Dotfile symlinks commonly point into a user's managed configuration checkout.
    let target = match fs::symlink_metadata(path) {
        Ok(info) if info.file_type().is_symlink() => fs::canonicalize(path)
            .map_err(|_| fault("Cannot resolve shell configuration symlink."))?,
        Ok(_) => path.to_path_buf(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => path.to_path_buf(),
        Err(_) => return Err(fault("Cannot inspect shell configuration.")),
    };
    let (mut text, mode) = match fs::read_to_string(&target) {
        Ok(text) => (
            text,
            fs::metadata(&target)
                .map_err(|_| fault("Cannot inspect shell configuration permissions."))?
                .permissions()
                .mode()
                & 0o777,
        ),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (String::new(), 0o600),
        Err(_) => return Err(fault("Cannot read shell configuration.")),
    };
    if !text.contains(PATH_SETUP) {
        if !text.is_empty() && !text.ends_with('\n') {
            text.push('\n');
        }
        text.push_str(PATH_SETUP);
    }
    Ok((target, text.into_bytes(), mode))
}

fn install_at(root: &Path, source: &Path, shells: &[PathBuf]) -> Result<PathBuf> {
    // Read everything before replacing the executable, including on a self-update.
    let binary = fs::read(source).map_err(|_| fault("Cannot read this executable."))?;
    let prepared = shells
        .iter()
        .map(|path| prepare_shell(path))
        .collect::<Result<Vec<_>>>()?;
    let executable = root.join(".local/bin/tvhome");
    write_atomic(
        &root.join(".tvhome/notices/LICENSES.txt"),
        LICENSES.as_bytes(),
        0o644,
    )?;
    write_atomic(&executable, &binary, 0o755)?;
    for (path, bytes, mode) in prepared {
        write_atomic(&path, &bytes, mode)?;
    }
    Ok(executable)
}

pub fn run() -> Result<()> {
    let root = PathBuf::from(std::env::var_os("HOME").ok_or(fault("HOME is unavailable."))?);
    if !root.is_absolute() {
        return Err(fault("HOME must be an absolute path."));
    }
    let source = std::env::current_exe().map_err(|_| fault("Cannot locate this executable."))?;
    let shell = PathBuf::from(std::env::var_os("SHELL").unwrap_or_default());
    let zdotdir = std::env::var_os("ZDOTDIR")
        .filter(|s| !s.is_empty())
        .map(PathBuf::from);
    let shells = shell_files(&root, &shell, zdotdir.as_deref(), cfg!(target_os = "macos"));
    let executable = install_at(&root, &source, &shells)?;
    println!(
        "Installed tvhome {} at {}",
        env!("CARGO_PKG_VERSION"),
        executable.display()
    );
    println!(
        "License notices: {}",
        root.join(".tvhome/notices/LICENSES.txt").display()
    );
    if shells.is_empty() {
        println!(
            "Shell PATH was not changed. Run {} --help",
            executable.display()
        );
    } else {
        for path in shells {
            println!("Configured PATH in {}", path.display());
        }
        println!("Open a new terminal, then run: tvhome --tv living-room setup");
        println!("In this terminal you can use: ~/.local/bin/tvhome --tv living-room setup");
    }
    Ok(())
}

pub fn update(root: &Path) -> Result<()> {
    let source = std::env::current_exe().map_err(|_| fault("Cannot locate this executable."))?;
    install_at(root, &source, &[])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn install_updates_atomically_and_is_repeatable() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("download");
        let shell = dir.path().join(".zshrc");
        fs::write(&source, b"first executable").unwrap();
        fs::write(&shell, "export EXISTING=value").unwrap();
        fs::set_permissions(&shell, fs::Permissions::from_mode(0o640)).unwrap();
        let installed = install_at(dir.path(), &source, std::slice::from_ref(&shell)).unwrap();
        let old = fs::File::open(&installed).unwrap();
        fs::write(&source, b"second executable").unwrap();
        install_at(dir.path(), &source, std::slice::from_ref(&shell)).unwrap();
        install_at(dir.path(), &installed, std::slice::from_ref(&shell)).unwrap();
        assert_eq!(fs::read(&installed).unwrap(), b"second executable");
        use std::io::Read;
        let mut previous = String::new();
        (&old).read_to_string(&mut previous).unwrap();
        assert_eq!(previous, "first executable");
        assert_eq!(
            fs::metadata(&installed).unwrap().permissions().mode() & 0o777,
            0o755
        );
        assert_eq!(
            fs::read_to_string(&shell).unwrap(),
            format!("export EXISTING=value\n{PATH_SETUP}")
        );
        assert_eq!(
            fs::metadata(&shell).unwrap().permissions().mode() & 0o777,
            0o640
        );
        assert_eq!(
            fs::read_to_string(dir.path().join(".tvhome/notices/LICENSES.txt")).unwrap(),
            LICENSES
        );
    }
    #[test]
    fn installed_binary_runs_after_copy() {
        let dir = tempfile::tempdir().unwrap();
        let installed = install_at(dir.path(), &std::env::current_exe().unwrap(), &[]).unwrap();
        let result = std::process::Command::new(installed)
            .arg("--list")
            .output()
            .unwrap();
        assert!(result.status.success());
        assert!(
            String::from_utf8_lossy(&result.stdout).contains("installed_binary_runs_after_copy")
        );
    }
    #[test]
    fn shell_selection_respects_zdotdir_and_bash_login_precedence() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let custom = root.join("dotfiles");
        assert_eq!(
            shell_files(root, Path::new("/bin/zsh"), Some(&custom), true),
            vec![custom.join(".zshrc")]
        );
        fs::write(root.join(".profile"), "existing").unwrap();
        assert_eq!(
            shell_files(root, Path::new("/bin/bash"), None, true),
            vec![root.join(".bashrc"), root.join(".profile")]
        );
        assert_eq!(
            shell_files(root, Path::new("/bin/bash"), None, false),
            vec![root.join(".bashrc")]
        );
        assert!(shell_files(root, Path::new("/bin/fish"), None, false).is_empty());
    }
    #[test]
    fn dotfile_symlinks_survive_and_invalid_configuration_preserves_install() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("download");
        fs::write(&source, "executable").unwrap();
        let managed = dir.path().join("managed-rc");
        fs::write(&managed, "# existing\n").unwrap();
        let link = dir.path().join(".zshrc");
        std::os::unix::fs::symlink(&managed, &link).unwrap();
        let installed = install_at(dir.path(), &source, std::slice::from_ref(&link)).unwrap();
        assert!(fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink());
        assert!(fs::read_to_string(&managed)
            .unwrap()
            .starts_with("# existing\n"));
        fs::write(&source, "replacement").unwrap();
        fs::write(&managed, [0xff, 0xfe]).unwrap();
        assert!(install_at(dir.path(), &source, &[link]).is_err());
        assert_eq!(fs::read_to_string(installed).unwrap(), "executable");
    }
}
