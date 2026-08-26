//! Linux desktop integrations: autostart, sleep inhibit, Send-to.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};

static STANDALONE_DIALOG: AtomicBool = AtomicBool::new(false);

const INVALID_NAME_CHARS: &str = r#"<>:"/\|?*"#;

pub fn sanitize_name(name: &str) -> String {
    name.chars()
        .map(|c| {
            if INVALID_NAME_CHARS.contains(c) {
                '-'
            } else {
                c
            }
        })
        .collect()
}

pub fn send_to_display_name(remote: &str, path: Option<&str>) -> String {
    let path_suffix = path
        .filter(|p| !p.is_empty() && *p != "/")
        .map(|p| {
            format!(
                " - {}",
                p.trim_start_matches('/').replace(['/', '\\'], " - ")
            )
        })
        .unwrap_or_default();
    sanitize_name(&format!("{remote}{path_suffix} (RClone Manager)"))
}

/// `nautilus.notifications.sendToAdded` interpolates `{{remote}}{{path}}`.
pub fn send_to_path_param(path: Option<&str>) -> String {
    path.filter(|p| !p.is_empty() && *p != "/")
        .map(|p| format!("/{}", p.trim_start_matches('/')))
        .unwrap_or_default()
}

fn apply_template(template: &str, replacements: &[(&str, &str)]) -> String {
    let mut content = template.to_string();
    for &(key, value) in replacements {
        content = content.replace(&format!("{{{key}}}"), value);
    }
    content
}

fn home_dir() -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| PathBuf::from("."))
}

fn write_executable(path: &Path, content: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, content)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(path)?.permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(path, perms)?;
    }
    Ok(())
}

pub fn current_exe_quoted() -> String {
    std::env::current_exe()
        .map(|p| format!("\"{}\"", p.display()))
        .unwrap_or_else(|_| "rclone-manager-gtk".into())
}

pub fn autostart_desktop_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| home_dir().join(".config"))
        .join("autostart/rclone-manager-gtk.desktop")
}

pub fn set_autostart(enabled: bool) -> Result<(), String> {
    if is_flatpak() {
        if let Err(err) = request_background_portal(enabled) {
            log::warn!("Flatpak Background portal failed, using XDG autostart: {err}");
        }
    }
    let path = autostart_desktop_path();
    if !enabled {
        let _ = std::fs::remove_file(&path);
        return Ok(());
    }
    let exec = current_exe_quoted();
    write_executable(&path, &autostart_desktop_entry(&exec)).map_err(|e| e.to_string())
}

/// XDG autostart entry. `--tray` matches Tauri `tauri_plugin_autostart`.
pub fn autostart_desktop_entry(exec: &str) -> String {
    format!(
        "[Desktop Entry]\nType=Application\nName=Rclone Manager\nComment=Manage rclone remotes, mounts, and transfers\nExec={exec} --tray\nIcon=folder-remote\nTerminal=false\nCategories=Network;FileTransfer;\nX-GNOME-Autostart-enabled=true\n"
    )
}

pub fn background_portal_commandline() -> Vec<String> {
    vec!["rclone-manager-gtk".into(), "--hidden".into()]
}

pub fn background_portal_options(
    enable: bool,
) -> std::collections::HashMap<String, dbus::arg::Variant<Box<dyn dbus::arg::RefArg>>> {
    let mut options = std::collections::HashMap::new();
    options.insert(
        "reason".into(),
        dbus::arg::Variant(Box::new(
            "Rclone Manager needs to run in the background to handle scheduled jobs and serve remotes."
                .to_string(),
        ) as Box<dyn dbus::arg::RefArg>),
    );
    options.insert(
        "autostart".into(),
        dbus::arg::Variant(Box::new(enable) as Box<dyn dbus::arg::RefArg>),
    );
    options.insert(
        "dbus-activatable".into(),
        dbus::arg::Variant(Box::new(false) as Box<dyn dbus::arg::RefArg>),
    );
    options.insert(
        "commandline".into(),
        dbus::arg::Variant(Box::new(background_portal_commandline()) as Box<dyn dbus::arg::RefArg>),
    );
    options
}

pub fn request_background_portal(enable: bool) -> Result<String, String> {
    let conn = dbus::blocking::Connection::new_session().map_err(|e| e.to_string())?;
    let proxy = conn.with_proxy(
        "org.freedesktop.portal.Desktop",
        "/org/freedesktop/portal/desktop",
        std::time::Duration::from_secs(8),
    );
    let options = background_portal_options(enable);
    let (path,): (dbus::strings::Path<'static>,) = proxy
        .method_call(
            "org.freedesktop.portal.Background",
            "RequestBackground",
            ("", options),
        )
        .map_err(|e| e.to_string())?;
    Ok(path.to_string())
}

pub fn autostart_enabled() -> bool {
    autostart_desktop_path().exists()
}

/// NetworkManager `Metered` values 1 (yes) and 3 (guess yes).
pub fn metered_from_nm_status(status: u32) -> bool {
    matches!(status, 1 | 3)
}

pub fn is_network_metered() -> bool {
    use std::time::Duration;
    let Ok(conn) = dbus::blocking::Connection::new_system() else {
        return false;
    };
    let proxy = conn.with_proxy(
        "org.freedesktop.NetworkManager",
        "/org/freedesktop/NetworkManager",
        Duration::from_millis(400),
    );
    use dbus::blocking::stdintf::org_freedesktop_dbus::Properties;
    match proxy.get::<u32>("org.freedesktop.NetworkManager", "Metered") {
        Ok(status) => metered_from_nm_status(status),
        Err(_) => false,
    }
}

pub fn is_flatpak() -> bool {
    std::env::var_os("FLATPAK_ID").is_some() || Path::new("/.flatpak-info").is_file()
}

/// Angular `about-modal` `updateInstructions` for managed Linux builds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManagedBuild {
    Flatpak,
    Portable,
    Default,
}

pub fn managed_build() -> ManagedBuild {
    if is_flatpak() {
        ManagedBuild::Flatpak
    } else if std::env::var_os("APPIMAGE").is_some() {
        ManagedBuild::Portable
    } else {
        ManagedBuild::Default
    }
}

pub fn update_command(build: ManagedBuild) -> Option<&'static str> {
    match build {
        ManagedBuild::Flatpak => Some("flatpak update io.github.zarestia_dev.rclone-manager"),
        ManagedBuild::Portable => None,
        ManagedBuild::Default => None,
    }
}

pub fn update_page_url(build: ManagedBuild) -> Option<&'static str> {
    match build {
        ManagedBuild::Flatpak => {
            Some("https://flathub.org/apps/io.github.zarestia_dev.rclone-manager")
        }
        ManagedBuild::Portable => {
            Some("https://hakanismail.info/zarestia/rclone-manager/downloads")
        }
        ManagedBuild::Default => None,
    }
}

struct LogindInhibit {
    _conn: dbus::blocking::Connection,
    _fd: dbus::arg::OwnedFd,
}

pub struct PowerInhibitor {
    child: Option<Child>,
    logind: Option<LogindInhibit>,
}

impl Default for PowerInhibitor {
    fn default() -> Self {
        Self::new()
    }
}

impl PowerInhibitor {
    pub fn new() -> Self {
        Self {
            child: None,
            logind: None,
        }
    }

    pub fn is_inhibited(&self) -> bool {
        self.child.is_some() || self.logind.is_some()
    }

    pub fn update(&mut self, should_inhibit: bool, reason: &str) {
        if should_inhibit {
            self.acquire(reason);
        } else {
            self.release();
        }
    }

    pub fn acquire(&mut self, reason: &str) {
        if self.is_inhibited() {
            return;
        }
        if let Some(hold) = acquire_logind(reason) {
            log::info!("logind power inhibitor acquired: {reason}");
            self.logind = Some(hold);
            return;
        }
        match Command::new("systemd-inhibit")
            .args([
                "--what=idle:sleep:shutdown",
                "--who=Rclone Manager",
                "--why",
                reason,
                "--mode=block",
                "sleep",
                "infinity",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        {
            Ok(child) => {
                log::info!("power inhibitor acquired: {reason}");
                self.child = Some(child);
            }
            Err(err) => log::warn!("systemd-inhibit unavailable: {err}"),
        }
    }

    pub fn release(&mut self) {
        if self.logind.take().is_some() {
            log::info!("logind power inhibitor released");
        }
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
            log::info!("power inhibitor released");
        }
    }
}

fn acquire_logind(reason: &str) -> Option<LogindInhibit> {
    use std::time::Duration;
    let conn = dbus::blocking::Connection::new_system().ok()?;
    let proxy = conn.with_proxy(
        "org.freedesktop.login1",
        "/org/freedesktop/login1",
        Duration::from_millis(1500),
    );
    let (fd,): (dbus::arg::OwnedFd,) = proxy
        .method_call(
            "org.freedesktop.login1.Manager",
            "Inhibit",
            ("idle:sleep:shutdown", "Rclone Manager", reason, "block"),
        )
        .ok()?;
    Some(LogindInhibit {
        _conn: conn,
        _fd: fd,
    })
}

impl Drop for PowerInhibitor {
    fn drop(&mut self) {
        self.release();
    }
}

const NAUTILUS_SCRIPT: &str = include_str!("../../src-tauri/resources/send_to/nautilus_script.sh");
const DOLPHIN_DESKTOP: &str =
    include_str!("../../src-tauri/resources/send_to/dolphin_action.desktop");
const NEMO_ACTION: &str = include_str!("../../src-tauri/resources/send_to/nemo_action.nemo_action");
const MACOS_INFO_PLIST: &str = include_str!("../../src-tauri/resources/send_to/macos_info.plist");
const MACOS_DOCUMENT_WFLOW: &str =
    include_str!("../../src-tauri/resources/send_to/macos_document.wflow");

pub fn register_send_to(remote: &str, path: Option<&str>) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        return register_windows_send_to(remote, path);
    }
    #[cfg(target_os = "macos")]
    {
        return register_macos_send_to(remote, path);
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        register_linux_send_to(remote, path)
    }
}

pub fn unregister_send_to(remote: &str, path: Option<&str>) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        return unregister_windows_send_to(remote, path);
    }
    #[cfg(target_os = "macos")]
    {
        return unregister_macos_send_to(remote, path);
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        unregister_linux_send_to(remote, path)
    }
}

pub fn is_send_to_registered(remote: &str, path: Option<&str>) -> bool {
    #[cfg(target_os = "windows")]
    {
        return is_windows_send_to_registered(remote, path);
    }
    #[cfg(target_os = "macos")]
    {
        return is_macos_send_to_registered(remote, path);
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        is_linux_send_to_registered(remote, path)
    }
}

fn register_linux_send_to(remote: &str, path: Option<&str>) -> Result<(), String> {
    let name = send_to_display_name(remote, path);
    let exec = current_exe_quoted();
    let path_val = path.unwrap_or("");
    let home = home_dir();
    let replacements = [
        ("exec_path", exec.as_str()),
        ("remote", remote),
        ("path", path_val),
        ("name", name.as_str()),
    ];

    write_executable(
        &home.join(".local/share/nautilus/scripts").join(&name),
        &apply_template(NAUTILUS_SCRIPT, &replacements),
    )
    .map_err(|e| e.to_string())?;
    write_executable(
        &home
            .join(".local/share/kio/servicemenus")
            .join(format!("{name}.desktop")),
        &apply_template(DOLPHIN_DESKTOP, &replacements),
    )
    .map_err(|e| e.to_string())?;
    write_executable(
        &home
            .join(".local/share/nemo/actions")
            .join(format!("{name}.nemo_action")),
        &apply_template(NEMO_ACTION, &replacements),
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

fn unregister_linux_send_to(remote: &str, path: Option<&str>) -> Result<(), String> {
    let name = send_to_display_name(remote, path);
    let home = home_dir();
    let _ = std::fs::remove_file(home.join(".local/share/nautilus/scripts").join(&name));
    let _ = std::fs::remove_file(
        home.join(".local/share/kio/servicemenus")
            .join(format!("{name}.desktop")),
    );
    let _ = std::fs::remove_file(
        home.join(".local/share/nemo/actions")
            .join(format!("{name}.nemo_action")),
    );
    Ok(())
}

fn is_linux_send_to_registered(remote: &str, path: Option<&str>) -> bool {
    let name = send_to_display_name(remote, path);
    let home = home_dir();
    home.join(".local/share/nautilus/scripts")
        .join(&name)
        .exists()
        || home
            .join(".local/share/kio/servicemenus")
            .join(format!("{name}.desktop"))
            .exists()
        || home
            .join(".local/share/nemo/actions")
            .join(format!("{name}.nemo_action"))
            .exists()
}

pub fn windows_sendto_dir() -> PathBuf {
    std::env::var("APPDATA")
        .map(|appdata| {
            PathBuf::from(appdata)
                .join("Microsoft")
                .join("Windows")
                .join("SendTo")
        })
        .unwrap_or_else(|_| home_dir().join("AppData/Roaming/Microsoft/Windows/SendTo"))
}

pub fn windows_sendto_arguments(remote: &str, path: &str) -> String {
    format!("--send-to-remote \"{remote}\" --send-to-path \"{path}\"")
}

pub fn windows_shortcut_ps1(exe: &str, arguments: &str, lnk: &str) -> String {
    let shortcut = lnk.replace('\'', "''");
    let target = exe.replace('\'', "''");
    let args = arguments.replace('\'', "''");
    format!(
        "$WshShell = New-Object -ComObject WScript.Shell; \
         $Shortcut = $WshShell.CreateShortcut('{shortcut}'); \
         $Shortcut.TargetPath = '{target}'; \
         $Shortcut.Arguments = '{args}'; \
         $Shortcut.IconLocation = '{target}'; \
         $Shortcut.Save()"
    )
}

pub fn windows_context_menu_command(exe: &str, remote: &str, path: &str) -> String {
    format!("\"{exe}\" --send-to-remote \"{remote}\" --send-to-path \"{path}\" \"%1\"")
}

pub fn windows_registry_ps1(
    name: &str,
    label: &str,
    exe: &str,
    remote: &str,
    path: &str,
) -> String {
    let command = windows_context_menu_command(exe, remote, path).replace('\'', "''");
    let name = name.replace('\'', "''");
    let label = label.replace('\'', "''");
    let icon = exe.replace('\'', "''");
    format!(
        "$roots = @('HKCU:\\Software\\Classes\\*','HKCU:\\Software\\Classes\\Directory'); \
         foreach ($root in $roots) {{ \
           $parent = Join-Path $root 'shell\\RCloneManager'; \
           New-Item -Path $parent -Force | Out-Null; \
           New-ItemProperty -Path $parent -Name 'MUIVerb' -Value 'RClone Manager' -Force | Out-Null; \
           New-ItemProperty -Path $parent -Name 'Icon' -Value '{icon}' -Force | Out-Null; \
           New-ItemProperty -Path $parent -Name 'SubCommands' -Value '' -Force | Out-Null; \
           $item = Join-Path $parent ('shell\\{name}'); \
           New-Item -Path $item -Force | Out-Null; \
           Set-ItemProperty -Path $item -Name '(default)' -Value '{label}'; \
           $cmd = Join-Path $item 'command'; \
           New-Item -Path $cmd -Force | Out-Null; \
           Set-ItemProperty -Path $cmd -Name '(default)' -Value '{command}'; \
         }}"
    )
}

fn windows_upload_label(remote: &str, path: &str) -> String {
    if path.is_empty() {
        format!("Upload to {remote}")
    } else {
        format!("Upload to {remote}/{}", path.trim_start_matches('/'))
    }
}

#[cfg(target_os = "windows")]
fn register_windows_send_to(remote: &str, path: Option<&str>) -> Result<(), String> {
    let name = send_to_display_name(remote, path);
    let path_val = path.unwrap_or("");
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let exe = exe.to_string_lossy();
    let dir = windows_sendto_dir();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let lnk = dir.join(format!("{name}.lnk"));
    let arguments = windows_sendto_arguments(remote, path_val);
    let shortcut = windows_shortcut_ps1(&exe, &arguments, &lnk.to_string_lossy());
    let registry = windows_registry_ps1(
        &name,
        &windows_upload_label(remote, path_val),
        &exe,
        remote,
        path_val,
    );
    run_powershell(&format!("{shortcut}; {registry}"))
}

#[cfg(target_os = "windows")]
fn unregister_windows_send_to(remote: &str, path: Option<&str>) -> Result<(), String> {
    let name = send_to_display_name(remote, path);
    let lnk = windows_sendto_dir().join(format!("{name}.lnk"));
    let _ = std::fs::remove_file(lnk);
    let name = name.replace('\'', "''");
    run_powershell(&format!(
        "$roots = @('HKCU:\\Software\\Classes\\*','HKCU:\\Software\\Classes\\Directory'); \
         foreach ($root in $roots) {{ \
           $item = Join-Path $root 'shell\\RCloneManager\\shell\\{name}'; \
           if (Test-Path $item) {{ Remove-Item -Path $item -Recurse -Force }}; \
           $parent = Join-Path $root 'shell\\RCloneManager'; \
           if ((Test-Path $parent) -and -not (Get-ChildItem (Join-Path $parent 'shell') -ErrorAction SilentlyContinue)) {{ \
             Remove-Item -Path $parent -Recurse -Force \
           }} \
         }}"
    ))
}

#[cfg(target_os = "windows")]
fn is_windows_send_to_registered(remote: &str, path: Option<&str>) -> bool {
    let name = send_to_display_name(remote, path);
    windows_sendto_dir().join(format!("{name}.lnk")).exists()
}

#[cfg(target_os = "windows")]
fn run_powershell(script: &str) -> Result<(), String> {
    let exe = if which::which("powershell").is_ok() {
        "powershell"
    } else if which::which("pwsh").is_ok() {
        "pwsh"
    } else {
        return Err("Neither powershell nor pwsh is available".into());
    };
    let output = Command::new(exe)
        .args(["-NoProfile", "-Command", script])
        .output()
        .map_err(|e| e.to_string())?;
    if output.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).into_owned())
    }
}

pub fn escape_xml(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

pub fn macos_workflow_dir(name: &str) -> PathBuf {
    home_dir().join(format!("Library/Services/{name}.workflow"))
}

pub fn macos_info_plist(name: &str, uuid: &str) -> String {
    apply_template(
        MACOS_INFO_PLIST,
        &[("uuid", uuid), ("name", &escape_xml(name))],
    )
}

pub fn macos_document_wflow(exe: &str, remote: &str, path: &str) -> String {
    let cmd = escape_xml(&format!(
        "exec \"{exe}\" --send-to-remote \"{remote}\" --send-to-path \"{path}\" \"$@\""
    ));
    apply_template(
        MACOS_DOCUMENT_WFLOW,
        &[
            ("cmd_string", cmd.as_str()),
            (
                "input_uuid",
                &uuid::Uuid::new_v4().to_string().to_uppercase(),
            ),
            (
                "output_uuid",
                &uuid::Uuid::new_v4().to_string().to_uppercase(),
            ),
            (
                "action_uuid",
                &uuid::Uuid::new_v4().to_string().to_uppercase(),
            ),
        ],
    )
}

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn write_macos_workflow(remote: &str, path: Option<&str>, exe: &Path) -> Result<PathBuf, String> {
    let name = send_to_display_name(remote, path);
    let workflow = macos_workflow_dir(&name);
    let contents = workflow.join("Contents");
    std::fs::create_dir_all(&contents).map_err(|e| e.to_string())?;
    std::fs::write(
        contents.join("Info.plist"),
        macos_info_plist(&name, &uuid::Uuid::new_v4().to_string().replace('-', "")),
    )
    .map_err(|e| e.to_string())?;
    std::fs::write(
        contents.join("document.wflow"),
        macos_document_wflow(&exe.to_string_lossy(), remote, path.unwrap_or("")),
    )
    .map_err(|e| e.to_string())?;
    Ok(workflow)
}

#[cfg(target_os = "macos")]
fn register_macos_send_to(remote: &str, path: Option<&str>) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    write_macos_workflow(remote, path, &exe).map(|_| ())
}

#[cfg(target_os = "macos")]
fn unregister_macos_send_to(remote: &str, path: Option<&str>) -> Result<(), String> {
    let workflow = macos_workflow_dir(&send_to_display_name(remote, path));
    if workflow.exists() {
        std::fs::remove_dir_all(workflow).map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn is_macos_send_to_registered(remote: &str, path: Option<&str>) -> bool {
    macos_workflow_dir(&send_to_display_name(remote, path)).exists()
}

#[derive(Debug, Clone, Default)]
pub struct SendToArgs {
    pub remote: String,
    pub path: String,
    pub files: Vec<PathBuf>,
}

pub fn parse_send_to_args(args: &[String]) -> Option<SendToArgs> {
    let mut remote = None;
    let mut path = String::new();
    let mut files = Vec::new();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--send-to-remote" => {
                i += 1;
                remote = args.get(i).cloned();
            }
            "--send-to-path" => {
                i += 1;
                path = args.get(i).cloned().unwrap_or_default();
            }
            other if other.starts_with('-') => {}
            _ if i == 0 => {}
            other => files.push(PathBuf::from(other)),
        }
        i += 1;
    }
    remote.map(|remote| SendToArgs {
        remote,
        path,
        files,
    })
}

/// Standalone dialog requested via `--dialog TYPE [--dialog-data JSON] [--dialog-result PATH]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DialogRequest {
    pub kind: String,
    pub data: serde_json::Value,
    pub result_path: Option<PathBuf>,
}

pub const DIALOG_KINDS: &[&str] = &[
    "preferences",
    "about",
    "logs",
    "export",
    "backend",
    "rclone-flags",
    "job-detail",
    "properties",
    "remote-about",
    "keyboard-shortcuts",
    "alerts",
    "archive-create",
    "quick-run-editor",
    "template-manager",
    "delete-remote",
    "remote-config",
    "quick-add-remote",
    "restore-preview",
    "vfs",
    "repair",
    "start-operation",
    "file-viewer",
];

pub fn parse_dialog_args(args: &[String]) -> Option<DialogRequest> {
    let mut kind = None;
    let mut data = serde_json::json!({});
    let mut result_path = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--dialog" => {
                i += 1;
                kind = args.get(i).cloned();
            }
            "--dialog-data" => {
                i += 1;
                if let Some(raw) = args.get(i) {
                    data = serde_json::from_str(raw).unwrap_or(serde_json::json!({}));
                }
            }
            "--dialog-result" => {
                i += 1;
                result_path = args.get(i).map(PathBuf::from);
            }
            _ => {}
        }
        i += 1;
    }
    kind.filter(|k| DIALOG_KINDS.contains(&k.as_str()))
        .map(|kind| DialogRequest {
            kind,
            data,
            result_path,
        })
}

pub fn set_standalone_dialog(enabled: bool) {
    STANDALONE_DIALOG.store(enabled, Ordering::SeqCst);
}

pub fn is_standalone_dialog() -> bool {
    STANDALONE_DIALOG.load(Ordering::SeqCst)
}

pub fn spawn_standalone_dialog(
    kind: &str,
    data: &serde_json::Value,
) -> Result<(Child, PathBuf), String> {
    if !DIALOG_KINDS.contains(&kind) {
        return Err(format!("unknown dialog type: {kind}"));
    }
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let result = std::env::temp_dir().join(format!(
        "rm-dialog-{}-{}.json",
        sanitize_name(kind),
        chrono::Utc::now().timestamp_millis()
    ));
    let child = Command::new(exe)
        .arg("--dialog")
        .arg(kind)
        .arg("--dialog-data")
        .arg(data.to_string())
        .arg("--dialog-result")
        .arg(&result)
        .stdin(Stdio::null())
        .spawn()
        .map_err(|e| e.to_string())?;
    Ok((child, result))
}

pub fn read_dialog_result(path: &Path) -> Option<serde_json::Value> {
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

pub fn write_dialog_result(
    path: Option<&Path>,
    ok: bool,
    kind: &str,
    data: serde_json::Value,
) -> Result<(), String> {
    let payload = serde_json::json!({
        "ok": ok,
        "type": kind,
        "data": data
    });
    match path {
        Some(path) => {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            std::fs::write(path, payload.to_string()).map_err(|e| e.to_string())
        }
        None => {
            println!("{payload}");
            Ok(())
        }
    }
}

/// `--share-intake FILE [FILE…]` queues local files for the Files upload banner.
pub fn parse_share_intake_args(args: &[String]) -> Option<Vec<PathBuf>> {
    let mut files = Vec::new();
    let mut found = false;
    let mut taking = false;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--share-intake" => {
                found = true;
                taking = true;
            }
            other if other.starts_with('-') => {
                taking = false;
                if matches!(
                    other,
                    "--send-to-remote"
                        | "--send-to-path"
                        | "--browse"
                        | "--browse-path"
                        | "--dialog"
                        | "--dialog-data"
                        | "--dialog-result"
                ) {
                    i += 1;
                }
            }
            _ if i == 0 => {}
            other if taking => files.push(PathBuf::from(other)),
            _ => {}
        }
        i += 1;
    }
    found.then_some(files)
}

pub fn enqueue_share_intake(files: &[PathBuf]) {
    let mut store = crate::store::AppStore::load();
    for file in files {
        let path = file.to_string_lossy().to_string();
        if path.is_empty() || store.pending_share_paths.iter().any(|p| p == &path) {
            continue;
        }
        store.pending_share_paths.push(path);
    }
    let _ = store.save();
}

pub fn file_uri(path: &Path) -> String {
    format!("file://{}", path.display())
}

pub fn share_portal_options(
) -> std::collections::HashMap<String, dbus::arg::Variant<Box<dyn dbus::arg::RefArg>>> {
    let mut options = std::collections::HashMap::new();
    options.insert(
        "handle_token".into(),
        dbus::arg::Variant(
            Box::new(format!("rclone{}", uuid::Uuid::new_v4().as_simple()))
                as Box<dyn dbus::arg::RefArg>,
        ),
    );
    options
}

pub fn request_share_portal(uri: &str) -> Result<String, String> {
    let conn = dbus::blocking::Connection::new_session().map_err(|e| e.to_string())?;
    let proxy = conn.with_proxy(
        "org.freedesktop.portal.Desktop",
        "/org/freedesktop/portal/desktop",
        std::time::Duration::from_secs(5),
    );
    let options = share_portal_options();
    let (path,): (dbus::strings::Path<'static>,) = proxy
        .method_call(
            "org.freedesktop.portal.Share",
            "ShareFile",
            ("", uri, options),
        )
        .map_err(|e| e.to_string())?;
    Ok(path.to_string())
}

/// Stage a local file for the desktop "share" action.
/// Prefers the xdg Share portal, then `xdg-email --attach`, then the default opener.
pub fn share_file(path: &Path) -> Result<(), String> {
    if !path.exists() {
        return Err(format!("File not found: {}", path.display()));
    }
    if request_share_portal(&file_uri(path)).is_ok() {
        return Ok(());
    }
    if Command::new("xdg-email")
        .arg("--attach")
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .is_ok()
    {
        return Ok(());
    }
    if Command::new("xdg-open")
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .is_ok()
    {
        return Ok(());
    }
    open::that(path).map_err(|e| e.to_string())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DebugInfo {
    pub logs_dir: String,
    pub config_dir: String,
    pub cache_dir: String,
    pub mode: String,
    pub app_version: String,
    pub platform: String,
    pub arch: String,
}

pub fn debug_info() -> DebugInfo {
    let config = crate::settings::AppSettings::config_dir();
    DebugInfo {
        logs_dir: crate::settings::AppSettings::log_path()
            .parent()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|| config.to_string_lossy().into_owned()),
        config_dir: config.to_string_lossy().into_owned(),
        cache_dir: crate::settings::AppSettings::cache_dir()
            .to_string_lossy()
            .into_owned(),
        mode: "gtk-desktop".into(),
        app_version: env!("CARGO_PKG_VERSION").into(),
        platform: std::env::consts::OS.into(),
        arch: std::env::consts::ARCH.into(),
    }
}

pub fn relaunch_command(current_exe: &Path, args: &[String]) -> std::process::Command {
    let mut cmd = Command::new(current_exe);
    cmd.args(args);
    cmd
}

pub fn relaunch() -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let args: Vec<String> = std::env::args().skip(1).collect();
    relaunch_command(&exe, &args)
        .spawn()
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn autostart_desktop_uses_tray_flag() {
        let entry = autostart_desktop_entry("\"/opt/rclone-manager-gtk\"");
        assert!(entry.contains("Exec=\"/opt/rclone-manager-gtk\" --tray"));
        assert!(entry.contains("X-GNOME-Autostart-enabled=true"));
    }

    #[test]
    fn background_portal_options_include_autostart() {
        let options = background_portal_options(true);
        assert!(options.contains_key("reason"));
        assert!(options.contains_key("autostart"));
        assert!(options.contains_key("commandline"));
        assert_eq!(
            background_portal_commandline(),
            vec!["rclone-manager-gtk".to_string(), "--hidden".to_string()]
        );
        let disabled = background_portal_options(false);
        assert!(disabled.contains_key("autostart"));
    }

    #[test]
    fn sanitizes_and_names_send_to() {
        assert_eq!(sanitize_name(r#"a/b:c"#), "a-b-c");
        assert_eq!(
            send_to_display_name("drive", Some("Photos/2024")),
            "drive - Photos - 2024 (RClone Manager)"
        );
        assert_eq!(
            send_to_display_name("drive", None),
            "drive (RClone Manager)"
        );
        assert_eq!(send_to_path_param(Some("Photos/2024")), "/Photos/2024");
        assert_eq!(send_to_path_param(Some("/")), "");
        assert_eq!(send_to_path_param(None), "");
    }

    #[test]
    fn parses_send_to_cli() {
        let args = [
            "rclone-manager-gtk".into(),
            "--send-to-remote".into(),
            "gdrive".into(),
            "--send-to-path".into(),
            "Inbox".into(),
            "/tmp/a.txt".into(),
            "/tmp/b.txt".into(),
        ];
        let parsed = parse_send_to_args(&args).unwrap();
        assert_eq!(parsed.remote, "gdrive");
        assert_eq!(parsed.path, "Inbox");
        assert_eq!(parsed.files.len(), 2);
        assert!(parse_send_to_args(&["app".into()]).is_none());
    }

    #[test]
    fn managed_build_update_instructions() {
        assert_eq!(
            update_command(ManagedBuild::Flatpak),
            Some("flatpak update io.github.zarestia_dev.rclone-manager")
        );
        assert!(update_page_url(ManagedBuild::Flatpak)
            .unwrap()
            .contains("flathub"));
        assert!(update_command(ManagedBuild::Default).is_none());
        assert!(update_page_url(ManagedBuild::Portable).is_some());
    }

    #[test]
    fn parses_share_intake_cli() {
        let args = [
            "rclone-manager-gtk".into(),
            "--share-intake".into(),
            "/tmp/a.jpg".into(),
            "/tmp/b.png".into(),
        ];
        let parsed = parse_share_intake_args(&args).unwrap();
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0], PathBuf::from("/tmp/a.jpg"));
        assert!(parse_share_intake_args(&["app".into()]).is_none());
        let mixed = [
            "app".into(),
            "--browse".into(),
            "testdrive:".into(),
            "--share-intake".into(),
            "/tmp/a.jpg".into(),
            "/tmp/b.png".into(),
        ];
        let parsed = parse_share_intake_args(&mixed).unwrap();
        assert_eq!(
            parsed,
            vec![PathBuf::from("/tmp/a.jpg"), PathBuf::from("/tmp/b.png")]
        );
    }

    #[test]
    fn parses_standalone_dialog_cli() {
        let args = [
            "app".into(),
            "--dialog".into(),
            "job-detail".into(),
            "--dialog-data".into(),
            r#"{"jobid":7,"remote":"drive"}"#.into(),
            "--dialog-result".into(),
            "/tmp/rm-dialog.json".into(),
        ];
        let parsed = parse_dialog_args(&args).unwrap();
        assert_eq!(parsed.kind, "job-detail");
        assert_eq!(parsed.data["jobid"], 7);
        assert_eq!(
            parsed.result_path,
            Some(PathBuf::from("/tmp/rm-dialog.json"))
        );
        assert!(parse_dialog_args(&["app".into(), "--dialog".into(), "nope".into()]).is_none());
        assert!(spawn_standalone_dialog("nope", &serde_json::json!({})).is_err());
        for kind in ["vfs", "repair", "start-operation", "file-viewer"] {
            let parsed =
                parse_dialog_args(&["app".into(), "--dialog".into(), kind.into()]).unwrap();
            assert_eq!(parsed.kind, kind);
        }
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("result.json");
        write_dialog_result(Some(&path), true, "about", serde_json::json!({})).unwrap();
        let text = std::fs::read_to_string(path).unwrap();
        assert!(text.contains("\"ok\":true"));
        assert!(text.contains("about"));
    }

    #[test]
    fn templates_substitute_placeholders() {
        let out = apply_template("x {remote} {path}", &[("remote", "a"), ("path", "b")]);
        assert_eq!(out, "x a b");
    }

    #[test]
    fn metered_status_matches_networkmanager() {
        assert!(metered_from_nm_status(1));
        assert!(metered_from_nm_status(3));
        assert!(!metered_from_nm_status(0));
        assert!(!metered_from_nm_status(2));
        assert!(!metered_from_nm_status(4));
    }

    #[test]
    fn share_missing_file_errors() {
        let err = share_file(Path::new("/tmp/rclone-manager-missing-share-file"))
            .expect_err("missing file");
        assert!(err.contains("not found"));
    }

    #[test]
    fn share_portal_options_include_handle_token() {
        let options = share_portal_options();
        assert!(options.contains_key("handle_token"));
        assert_eq!(file_uri(Path::new("/tmp/a.txt")), "file:///tmp/a.txt");
    }

    #[test]
    fn power_inhibitor_release_is_idempotent() {
        let mut inhibitor = PowerInhibitor::new();
        assert!(!inhibitor.is_inhibited());
        inhibitor.release();
        inhibitor.update(false, "idle");
        assert!(!inhibitor.is_inhibited());
    }

    #[test]
    fn debug_info_has_gtk_mode_and_version() {
        let info = debug_info();
        assert_eq!(info.mode, "gtk-desktop");
        assert_eq!(info.app_version, env!("CARGO_PKG_VERSION"));
        assert!(!info.config_dir.is_empty());
        assert!(!info.platform.is_empty());
        assert!(!info.arch.is_empty());
    }

    #[test]
    fn relaunch_command_keeps_exe_and_args() {
        let cmd = relaunch_command(Path::new("/opt/rclone-manager-gtk"), &["--foo".into()]);
        assert_eq!(cmd.get_program(), "/opt/rclone-manager-gtk");
        let args: Vec<_> = cmd.get_args().collect();
        assert_eq!(args, vec!["--foo"]);
    }

    #[test]
    fn windows_send_to_scripts_quote_paths() {
        let args = windows_sendto_arguments("gdrive", "Inbox");
        assert!(args.contains("--send-to-remote \"gdrive\""));
        let ps1 = windows_shortcut_ps1(
            r"C:\Program Files\app.exe",
            &args,
            r"C:\Users\me\SendTo\drive.lnk",
        );
        assert!(ps1.contains("CreateShortcut"));
        assert!(ps1.contains("TargetPath"));
        assert!(!ps1.contains("'{target}')"));
        let cmd = windows_context_menu_command(r"C:\app.exe", "box", "Photos");
        assert!(cmd.contains("\"%1\""));
        let reg = windows_registry_ps1(
            "box (RClone Manager)",
            "Upload to box",
            r"C:\app.exe",
            "box",
            "",
        );
        assert!(reg.contains("HKCU:\\Software\\Classes\\*"));
        assert!(reg.contains("RCloneManager"));
    }

    #[test]
    fn macos_workflow_templates_escape_and_substitute() {
        let plist = macos_info_plist("drive & photos", "abc123");
        assert!(plist.contains("drive &amp; photos"));
        assert!(plist.contains("abc123"));
        let wflow = macos_document_wflow(
            "/Applications/Rclone Manager.app/Contents/MacOS/app",
            "gdrive",
            "Inbox",
        );
        assert!(wflow.contains("--send-to-remote"));
        assert!(wflow.contains("gdrive"));
        assert!(wflow.contains("Inbox"));
        assert!(macos_workflow_dir("drive (RClone Manager)")
            .to_string_lossy()
            .contains("Library/Services/drive (RClone Manager).workflow"));
    }
}
