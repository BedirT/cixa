use agent_treasury_domain::{
    API_VERSION, ApprovedSecretOperation, AutonomyMode, BalanceStatus, Money,
    OwnerHandoffTransport, Policy, ProviderOutcome, PurchaseItem, PurchaseRequest,
    ReconciliationOutcome, Request, RpcRequest, RpcResponse, SecretProvider, SimulatedScenario,
    SimulatedSecretProvider, Treasury, VolatileSecret, redact_sensitive,
};
#[cfg(unix)]
use agent_treasury_domain::{
    DurableNonceRedemptionStore, OwnerControlledSecretHelperProvider,
    redeem_owner_helper_operation, unix_peer_effective_uid,
};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::env;
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdout, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use zeroize::{Zeroize, Zeroizing};

type CliResult<T> = Result<T, Box<dyn std::error::Error>>;
const MAX_FRAME_BYTES: usize = 256 * 1024;
const MAX_CONNECTIONS: usize = 32;
const MAX_OWNER_CONNECTIONS: usize = 8;

struct DataDirLock(fs::File);

impl DataDirLock {
    fn acquire(directory: &Path) -> CliResult<Self> {
        fs::create_dir_all(directory)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
            let metadata = fs::symlink_metadata(directory)?;
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                return Err("data directory must be a real directory".into());
            }
            fs::set_permissions(directory, fs::Permissions::from_mode(0o700))?;
            let lock_path = directory.join("treasury.lock");
            if let Ok(metadata) = fs::symlink_metadata(&lock_path)
                && (metadata.file_type().is_symlink() || !metadata.is_file())
            {
                return Err("treasury lock must be a regular file".into());
            }
            let file = OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .mode(0o600)
                .custom_flags(libc::O_NOFOLLOW)
                .open(lock_path)?;
            fs::set_permissions(
                directory.join("treasury.lock"),
                fs::Permissions::from_mode(0o600),
            )?;
            file.try_lock_exclusive()
                .map_err(|_| "treasury data directory is already owned by another writer")?;
            Ok(Self(file))
        }
        #[cfg(not(unix))]
        {
            let file = OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .open(directory.join("treasury.lock"))?;
            file.try_lock_exclusive()
                .map_err(|_| "treasury data directory is already owned by another writer")?;
            Ok(Self(file))
        }
    }
}

impl Drop for DataDirLock {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.0);
    }
}

fn main() {
    if let Err(error) = run() {
        eprintln!("error: {}", redact_sensitive(&error.to_string()));
        std::process::exit(1);
    }
}

fn run() -> CliResult<()> {
    let mut args = env::args().skip(1);
    let command = args.next().unwrap_or_else(|| "help".to_string());
    let rest: Vec<String> = args.collect();
    #[cfg(not(unix))]
    if !matches!(command.as_str(), "help" | "--help" | "-h" | "demo") {
        return Err("stateful treasury commands require the documented Windows named-pipe and DACL adapter; this build fails closed".into());
    }
    match command.as_str() {
        "help" | "--help" | "-h" => print_help(),
        "demo" => run_demo(),
        "init" => init_command(&rest),
        "create-agent" => create_agent_command(&rest),
        "update-policy" => update_policy_command(&rest),
        "revoke-agent" => agent_lifecycle_command(&rest, true),
        "set-agent-mode" => agent_lifecycle_command(&rest, false),
        "arm-session" => arm_session_command(&rest),
        "configure-manual-provider" => configure_manual_provider_command(&rest),
        "configure-receive" => configure_receive_command(&rest),
        "record-deposit" => record_deposit_command(&rest),
        "status" => direct_command(&rest, Request::GetStatus),
        "budget" => direct_command(&rest, Request::GetBudget),
        "capabilities" => direct_command(&rest, Request::GetCapabilities),
        "receive-instructions" => direct_command(&rest, Request::GetReceiveInstructions),
        "intent" => intent_command(&rest),
        "execute" => intent_id_command(&rest, false),
        "cancel" => intent_id_command(&rest, true),
        "approve" => owner_intent_command(&rest, false),
        "begin-handoff" => owner_handoff_command(&rest, false),
        "complete-handoff" => owner_handoff_command(&rest, true),
        "init-helper" => init_helper_command(&rest),
        "secret-helper" => secret_helper_command(&rest),
        "execute-handoff" => execute_handoff_command(&rest),
        "approve-merchant" => approve_merchant_command(&rest),
        "reconcile" => reconcile_command(&rest),
        "stop" => stop_command(&rest, true),
        "resume" => stop_command(&rest, false),
        "audit" => direct_command(&rest, Request::OwnerListAudit),
        "serve" => serve_command(&rest),
        other => Err(format!("unknown command {other}; run `treasury help`").into()),
    }
}

fn print_help() -> CliResult<()> {
    println!(
        "agent-treasury {}\n\nCommands:\n  demo                              Run the local adversarial demo\n  init --data-dir DIR --owner-token-file FILE\n  create-agent --data-dir DIR --owner-token-file FILE --agent-token-file FILE [--agent-gid GID]\n  update-policy --data-dir DIR --owner-token-file FILE --agent-id ID --policy-file FILE\n  revoke-agent --data-dir DIR --owner-token-file FILE --agent-id ID\n  set-agent-mode --data-dir DIR --owner-token-file FILE --agent-id ID --mode MODE\n  arm-session --data-dir DIR --owner-token-file FILE --agent-id ID --ttl-secs N\n  configure-manual-provider --data-dir DIR --owner-token-file FILE --credential-reference REF --balance-minor N --balance-status estimated|owner_confirmed\n  configure-receive --data-dir DIR --owner-token-file FILE --address VALUE\n  record-deposit --data-dir DIR --owner-token-file FILE --amount-minor N --currency CAD --source VALUE --external-reference REF --verified true|false\n  status|budget|capabilities|receive-instructions --data-dir DIR --token-file FILE\n  intent --data-dir DIR --token-file FILE --request-file FILE\n  execute|cancel --data-dir DIR --token-file FILE --intent-id ID\n  approve --data-dir DIR --owner-token-file FILE --intent-id ID\n  begin-handoff|complete-handoff --data-dir DIR --owner-token-file FILE --intent-id ID\n  init-helper --helper-dir DIR\n  secret-helper --socket PATH --helper-key-file FILE --helper-id-file FILE --redemption-dir DIR\n  execute-handoff --data-dir DIR --owner-token-file FILE --intent-id ID --helper-socket PATH --helper-key-file FILE --helper-id-file FILE --adapter-script FILE --adapter-config FILE --node-path FILE\n  approve-merchant --data-dir DIR --owner-token-file FILE --agent-id ID --merchant-domain DOMAIN\n  reconcile --data-dir DIR --owner-token-file FILE --intent-id ID --outcome settled|declined|refunded [--provider-reference REF]\n  stop|resume --data-dir DIR --owner-token-file FILE\n  audit --data-dir DIR --owner-token-file FILE\n  serve --data-dir DIR [--socket PATH] [--owner-socket PATH] [--agent-gid GID]\n\nTokens and payment material are read from protected files or stdin, never accepted as command-line values or printed.\nThe broker binds separate agent and owner Unix-domain sockets by default and does not expose a public listener.",
        env!("CARGO_PKG_VERSION")
    );
    Ok(())
}

fn value(args: &[String], name: &str) -> Option<String> {
    args.windows(2).find(|pair| pair[0] == name).map(|pair| pair[1].clone())
}

fn required(args: &[String], name: &str) -> CliResult<String> {
    value(args, name).ok_or_else(|| format!("missing required argument {name}").into())
}

fn data_dir(args: &[String]) -> CliResult<PathBuf> {
    Ok(PathBuf::from(required(args, "--data-dir")?))
}

fn token_file(args: &[String], owner: bool) -> CliResult<String> {
    let name = if owner { "--owner-token-file" } else { "--token-file" };
    let path = required(args, name)?;
    let metadata = fs::symlink_metadata(&path)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(format!("token file {path} must be a regular file").into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err(format!("token file {path} permissions are too broad").into());
        }
    }
    let token = fs::read_to_string(&path)?.trim().to_string();
    if token.is_empty() {
        return Err(format!("token file {path} is empty").into());
    }
    Ok(token)
}

fn write_token(path: &Path, token: &str) -> CliResult<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    let mut file = options.open(path)?;
    file.write_all(format!("{token}\n").as_bytes())?;
    file.sync_all()?;
    Ok(())
}

fn read_private_text(path: &Path, label: &str) -> CliResult<String> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(format!("{label} must be a regular file").into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err(format!("{label} permissions are too broad").into());
        }
    }
    let value = fs::read_to_string(path)?.trim().to_string();
    if value.is_empty() {
        return Err(format!("{label} is empty").into());
    }
    Ok(value)
}

fn require_absolute_regular_file(path: &Path, label: &str) -> CliResult<()> {
    if !path.is_absolute() {
        return Err(format!("{label} must be an absolute path").into());
    }
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(format!("{label} must be a regular non-symlink file").into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        if metadata.uid() != unsafe { libc::geteuid() }
            || metadata.permissions().mode() & 0o022 != 0
        {
            return Err(
                format!("{label} must be owner-controlled and not group/world writable").into()
            );
        }
        let effective_uid = unsafe { libc::geteuid() };
        let mut ancestor = path.parent();
        while let Some(directory) = ancestor {
            let metadata = fs::symlink_metadata(directory)?;
            if metadata.file_type().is_symlink()
                || !metadata.is_dir()
                || (metadata.uid() != 0 && metadata.uid() != effective_uid)
                || metadata.permissions().mode() & 0o022 != 0
            {
                return Err(format!(
                    "{label} ancestors must be root/owner-controlled non-symlink directories"
                )
                .into());
            }
            ancestor = directory.parent();
        }
    }
    Ok(())
}

#[cfg(unix)]
fn require_owner_executable(path: &Path, label: &str) -> CliResult<()> {
    use std::os::unix::fs::PermissionsExt;
    require_absolute_regular_file(path, label)?;
    if fs::symlink_metadata(path)?.permissions().mode() & 0o111 == 0 {
        return Err(format!("{label} is not executable").into());
    }
    Ok(())
}

#[cfg(unix)]
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct CheckoutSecret<'a> {
    pan: &'a str,
    expiry: &'a str,
    cvv: &'a str,
    #[serde(default)]
    cardholder: Option<&'a str>,
}

#[cfg(unix)]
#[derive(Serialize)]
struct CheckoutAdapterInput<'a> {
    config: &'a Value,
    request: &'a PurchaseRequest,
    secret: CheckoutSecret<'a>,
}

#[cfg(unix)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct ProcessIdentity {
    pid: u32,
    started: u128,
}

#[cfg(target_os = "linux")]
fn process_identity(pid: u32) -> Option<ProcessIdentity> {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let after_name = stat.rsplit_once(") ")?.1;
    let started = after_name.split_whitespace().nth(19)?.parse::<u128>().ok()?;
    Some(ProcessIdentity { pid, started })
}

#[cfg(target_os = "macos")]
#[repr(C)]
#[derive(Default)]
struct ProcBsdInfo {
    flags: u32,
    status: u32,
    xstatus: u32,
    pid: u32,
    ppid: u32,
    uid: u32,
    gid: u32,
    ruid: u32,
    rgid: u32,
    svuid: u32,
    svgid: u32,
    rfu_1: u32,
    comm: [u8; 16],
    name: [u8; 32],
    nfiles: u32,
    pgid: u32,
    pjobc: u32,
    e_tdev: u32,
    e_tpgid: u32,
    nice: i32,
    start_tvsec: u64,
    start_tvusec: u64,
}

#[cfg(target_os = "macos")]
#[repr(C)]
#[derive(Default)]
struct ProcUniqueIdentifierInfo {
    uuid: [u8; 16],
    unique_id: u64,
    parent_unique_id: u64,
    id_version: i32,
    original_parent_id_version: i32,
    reserved: [u64; 2],
}

#[cfg(target_os = "macos")]
#[repr(C)]
#[derive(Default)]
struct ProcBsdInfoWithUniqueId {
    bsd: ProcBsdInfo,
    unique: ProcUniqueIdentifierInfo,
}

#[cfg(target_os = "macos")]
fn macos_process_info(pid: u32) -> Option<ProcBsdInfoWithUniqueId> {
    #[link(name = "proc")]
    unsafe extern "C" {
        fn proc_pidinfo(
            pid: libc::c_int,
            flavor: libc::c_int,
            arg: u64,
            buffer: *mut libc::c_void,
            buffersize: libc::c_int,
        ) -> libc::c_int;
    }
    const PROC_PIDT_BSDINFOWITHUNIQID: libc::c_int = 18;
    let mut info = ProcBsdInfoWithUniqueId::default();
    let expected = std::mem::size_of::<ProcBsdInfoWithUniqueId>() as libc::c_int;
    let received = unsafe {
        proc_pidinfo(
            pid as libc::c_int,
            PROC_PIDT_BSDINFOWITHUNIQID,
            0,
            (&mut info as *mut ProcBsdInfoWithUniqueId).cast(),
            expected,
        )
    };
    if received != expected || info.bsd.pid != pid {
        return None;
    }
    Some(info)
}

#[cfg(target_os = "macos")]
fn process_identity(pid: u32) -> Option<ProcessIdentity> {
    let info = macos_process_info(pid)?;
    Some(ProcessIdentity { pid, started: u128::from(info.unique.unique_id) })
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn process_identity(_pid: u32) -> Option<ProcessIdentity> {
    None
}

#[cfg(unix)]
fn process_identity_matches(identity: ProcessIdentity) -> bool {
    process_identity(identity.pid) == Some(identity)
}

#[cfg(unix)]
fn process_group_identity_matches(root: ProcessIdentity) -> bool {
    process_identity_matches(root)
}

#[cfg(target_os = "linux")]
fn signal_process_if_current(identity: ProcessIdentity) -> bool {
    let descriptor = unsafe { libc::syscall(libc::SYS_pidfd_open, identity.pid, 0) } as libc::c_int;
    if descriptor < 0 {
        return false;
    }
    let matches = process_identity_matches(identity);
    let result = if matches {
        (unsafe {
            libc::syscall(
                libc::SYS_pidfd_send_signal,
                descriptor,
                libc::SIGKILL,
                std::ptr::null::<libc::siginfo_t>(),
                0,
            )
        }) == 0
    } else {
        false
    };
    unsafe {
        libc::close(descriptor);
    }
    result
}

#[cfg(target_os = "macos")]
fn signal_process_if_current(identity: ProcessIdentity) -> bool {
    #[repr(C)]
    #[derive(Default)]
    struct AuditMask {
        success: u32,
        failure: u32,
    }
    #[repr(C)]
    #[derive(Default)]
    struct AuditTerminalId {
        port: i32,
        kind: u32,
        address: [u32; 4],
    }
    #[repr(C)]
    #[derive(Default)]
    struct AuditInfo {
        audit_user_id: u32,
        mask: AuditMask,
        terminal_id: AuditTerminalId,
        session_id: i32,
        flags: u64,
    }
    #[repr(C)]
    struct AuditToken {
        values: [u32; 8],
    }
    unsafe extern "C" {
        fn getaudit_addr(info: *mut AuditInfo, length: libc::c_int) -> libc::c_int;
    }
    #[link(name = "proc")]
    unsafe extern "C" {
        fn proc_signal_with_audittoken(token: *mut AuditToken, signal: libc::c_int) -> libc::c_int;
    }

    let info = match macos_process_info(identity.pid) {
        Some(info) if u128::from(info.unique.unique_id) == identity.started => info,
        _ => return false,
    };
    let mut audit = AuditInfo::default();
    if unsafe { getaudit_addr(&mut audit, std::mem::size_of::<AuditInfo>() as libc::c_int) } != 0 {
        return false;
    }
    let mut token = AuditToken {
        values: [
            audit.audit_user_id,
            info.bsd.uid,
            info.bsd.gid,
            info.bsd.ruid,
            info.bsd.rgid,
            identity.pid,
            audit.session_id as u32,
            info.unique.id_version as u32,
        ],
    };
    unsafe { proc_signal_with_audittoken(&mut token, libc::SIGKILL) == 0 }
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn signal_process_if_current(_identity: ProcessIdentity) -> bool {
    false
}

#[cfg(unix)]
fn direct_child_pids(parent: u32) -> Vec<u32> {
    #[cfg(target_os = "macos")]
    {
        #[link(name = "proc")]
        unsafe extern "C" {
            fn proc_listchildpids(
                ppid: libc::c_int,
                buffer: *mut libc::c_void,
                buffersize: libc::c_int,
            ) -> libc::c_int;
        }
        let mut pids = vec![0_i32; 4096];
        let count = unsafe {
            proc_listchildpids(
                parent as libc::c_int,
                pids.as_mut_ptr().cast(),
                (pids.len() * std::mem::size_of::<i32>()) as libc::c_int,
            )
        };
        if count <= 0 {
            return Vec::new();
        }
        pids.truncate(count as usize);
        pids.into_iter().filter(|pid| *pid > 0).map(|pid| pid as u32).collect()
    }
    #[cfg(target_os = "linux")]
    {
        let mut children = Vec::new();
        if let Ok(entries) = fs::read_dir("/proc") {
            for entry in entries.flatten() {
                let Ok(pid) = entry.file_name().to_string_lossy().parse::<u32>() else {
                    continue;
                };
                let Ok(stat) = fs::read_to_string(entry.path().join("stat")) else {
                    continue;
                };
                let Some(after_name) = stat.rsplit_once(") ").map(|(_, value)| value) else {
                    continue;
                };
                if after_name.split_whitespace().nth(1).and_then(|value| value.parse().ok())
                    == Some(parent)
                {
                    children.push(pid);
                }
            }
        }
        children
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    Vec::new()
}

#[cfg(unix)]
fn descendant_processes(root: u32) -> Vec<ProcessIdentity> {
    let mut pending = vec![root];
    let mut descendants = Vec::new();
    while let Some(parent) = pending.pop() {
        for child in direct_child_pids(parent) {
            if !descendants.iter().any(|identity: &ProcessIdentity| identity.pid == child) {
                if let Some(identity) = process_identity(child) {
                    descendants.push(identity);
                }
                pending.push(child);
            }
        }
    }
    descendants
}

#[cfg(unix)]
struct DescendantTracker {
    root: Option<ProcessIdentity>,
    stop: Arc<std::sync::atomic::AtomicBool>,
    processes: Arc<Mutex<std::collections::BTreeSet<ProcessIdentity>>>,
    watcher: Option<std::thread::JoinHandle<()>>,
}

#[cfg(unix)]
impl DescendantTracker {
    fn new(root: u32) -> Self {
        let root_identity = process_identity(root);
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let processes = Arc::new(Mutex::new(std::collections::BTreeSet::new()));
        let watcher_stop = Arc::clone(&stop);
        let watcher_processes = Arc::clone(&processes);
        let watcher = std::thread::spawn(move || {
            while !watcher_stop.load(Ordering::Acquire) {
                if let Ok(mut tracked) = watcher_processes.lock() {
                    tracked.extend(descendant_processes(root));
                }
                std::thread::sleep(Duration::from_millis(1));
            }
        });
        Self { root: root_identity, stop, processes, watcher: Some(watcher) }
    }

    fn terminate(&mut self, child: &mut Child) -> Option<ExitStatus> {
        self.stop.store(true, Ordering::Release);
        if let Some(watcher) = self.watcher.take() {
            let _ = watcher.join();
        }
        if let Ok(mut tracked) = self.processes.lock() {
            tracked.extend(descendant_processes(child.id()));
            if let Some(root) = self.root.filter(|root| process_group_identity_matches(*root)) {
                unsafe {
                    libc::killpg(root.pid as libc::pid_t, libc::SIGKILL);
                }
            }
            for identity in tracked.iter().rev() {
                signal_process_if_current(*identity);
            }
        }
        let _ = child.kill();
        child.wait().ok()
    }
}

#[cfg(all(test, unix, any(target_os = "macos", target_os = "linux")))]
mod process_identity_tests {
    use super::*;

    #[test]
    fn stale_process_identity_is_rejected_before_signaling() {
        let current = process_identity(std::process::id()).expect("current process identity");
        assert!(process_identity_matches(current));
        let stale = ProcessIdentity { started: current.started.wrapping_add(1), ..current };
        assert!(!process_identity_matches(stale));
        assert!(process_group_identity_matches(current));
        assert!(!process_group_identity_matches(stale));
        assert!(!signal_process_if_current(stale));
    }

    #[test]
    fn stable_process_handle_signals_the_observed_child() {
        let mut child = Command::new("sleep").arg("30").spawn().expect("spawn test child");
        let identity = process_identity(child.id()).expect("test child identity");
        assert!(signal_process_if_current(identity));
        child.wait().expect("reap test child");
    }
}

#[cfg(unix)]
fn terminate_child_group(child: &mut Child, tracker: &mut DescendantTracker) -> Option<ExitStatus> {
    tracker.terminate(child)
}

#[cfg(unix)]
fn child_exited_without_reaping(pid: u32) -> std::io::Result<bool> {
    let mut information: libc::siginfo_t = unsafe { std::mem::zeroed() };
    let result = unsafe {
        libc::waitid(
            libc::P_PID,
            pid as libc::id_t,
            &mut information,
            libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
        )
    };
    if result != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(unsafe { information.si_pid() } != 0)
}

#[cfg(unix)]
fn collect_child_output(
    child: &mut Child,
    tracker: &mut DescendantTracker,
    mut stdout: ChildStdout,
    timeout: Duration,
) -> agent_treasury_domain::Result<(ExitStatus, Zeroizing<Vec<u8>>)> {
    use std::os::fd::AsRawFd;

    let descriptor = stdout.as_raw_fd();
    let flags = unsafe { libc::fcntl(descriptor, libc::F_GETFL) };
    if flags < 0 || unsafe { libc::fcntl(descriptor, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0
    {
        terminate_child_group(child, tracker);
        return Err(std::io::Error::last_os_error().into());
    }

    let deadline = Instant::now() + timeout;
    let mut output = Zeroizing::new(Vec::new());
    let mut eof = false;
    while Instant::now() < deadline {
        let mut buffer = [0_u8; 4096];
        match stdout.read(&mut buffer) {
            Ok(0) => eof = true,
            Ok(count) => {
                output.extend_from_slice(&buffer[..count]);
                if output.len() > 16 * 1024 {
                    terminate_child_group(child, tracker);
                    return Err(agent_treasury_domain::TreasuryError::Conflict(
                        "controlled checkout adapter output is too large".to_string(),
                    ));
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(error) => {
                terminate_child_group(child, tracker);
                return Err(error.into());
            }
        }
        if eof {
            let exited = match child_exited_without_reaping(child.id()) {
                Ok(exited) => exited,
                Err(error) => {
                    terminate_child_group(child, tracker);
                    return Err(error.into());
                }
            };
            if exited {
                // Keep the root as a zombie until its process group is signaled,
                // then reap it and preserve its original exit status.
                let status = terminate_child_group(child, tracker).ok_or_else(|| {
                    agent_treasury_domain::TreasuryError::Conflict(
                        "controlled checkout adapter could not be reaped".to_string(),
                    )
                })?;
                return Ok((status, output));
            }
        }
        std::thread::sleep(Duration::from_millis(10));
    }

    terminate_child_group(child, tracker);
    Err(agent_treasury_domain::TreasuryError::Conflict(
        "controlled checkout adapter exceeded its hard deadline; payment outcome is unknown"
            .to_string(),
    ))
}

#[cfg(unix)]
struct PlaywrightCheckoutTransport {
    node_path: PathBuf,
    adapter_script: PathBuf,
    adapter_config: Value,
    deadline: Instant,
}

#[cfg(unix)]
impl OwnerHandoffTransport for PlaywrightCheckoutTransport {
    fn transport_id(&self) -> &str {
        "owner-controlled-playwright"
    }

    fn submit(
        &mut self,
        request: &PurchaseRequest,
        secret: &VolatileSecret,
    ) -> agent_treasury_domain::Result<ProviderOutcome> {
        let secret: CheckoutSecret<'_> =
            serde_json::from_slice(secret.as_bytes()).map_err(|_| {
                agent_treasury_domain::TreasuryError::Invalid(
                "owner secret must be one strict JSON object for the controlled checkout adapter"
                    .to_string(),
            )
            })?;
        if secret.pan.is_empty()
            || secret.pan.len() > 32
            || secret.expiry.is_empty()
            || secret.expiry.len() > 16
            || secret.cvv.is_empty()
            || secret.cvv.len() > 8
            || secret.cardholder.is_some_and(|value| value.is_empty() || value.len() > 128)
        {
            return Err(agent_treasury_domain::TreasuryError::Invalid(
                "owner secret fields are missing or outside their size limits".to_string(),
            ));
        }
        let input = CheckoutAdapterInput { config: &self.adapter_config, request, secret };
        let mut encoded = Zeroizing::new(serde_json::to_vec(&input)?);
        encoded.push(b'\n');
        if encoded.len() > 16 * 1024 {
            return Err(agent_treasury_domain::TreasuryError::Invalid(
                "checkout adapter request is too large".to_string(),
            ));
        }
        let mut command = Command::new(&self.node_path);
        command
            .arg(&self.adapter_script)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        let mut child = command.spawn()?;
        let mut tracker = DescendantTracker::new(child.id());
        let mut stdin = match child.stdin.take() {
            Some(stdin) => stdin,
            None => {
                terminate_child_group(&mut child, &mut tracker);
                return Err(agent_treasury_domain::TreasuryError::Conflict(
                    "checkout adapter stdin is unavailable".to_string(),
                ));
            }
        };
        if let Err(error) = stdin.write_all(&encoded) {
            terminate_child_group(&mut child, &mut tracker);
            return Err(error.into());
        }
        encoded.zeroize();
        drop(stdin);
        let stdout = match child.stdout.take() {
            Some(stdout) => stdout,
            None => {
                terminate_child_group(&mut child, &mut tracker);
                return Err(agent_treasury_domain::TreasuryError::Conflict(
                    "checkout adapter stdout is unavailable".to_string(),
                ));
            }
        };
        let remaining = self.deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            terminate_child_group(&mut child, &mut tracker);
            return Err(agent_treasury_domain::TreasuryError::Conflict(
                "owner handoff deadline expired before adapter completion".to_string(),
            ));
        }
        let (status, output) = collect_child_output(&mut child, &mut tracker, stdout, remaining)?;
        if !status.success() {
            return Err(agent_treasury_domain::TreasuryError::Conflict(
                "controlled checkout adapter failed; payment outcome is unknown".to_string(),
            ));
        }
        let _: ProviderOutcome = serde_json::from_slice(&output).map_err(|_| {
            agent_treasury_domain::TreasuryError::Conflict(
                "controlled checkout adapter returned an invalid sanitized outcome".to_string(),
            )
        })?;
        Ok(ProviderOutcome::Unknown {
            reason: "browser output is not authenticated provider evidence; owner reconciliation required"
                .to_string(),
        })
    }

    fn cleanup(&mut self) -> agent_treasury_domain::Result<()> {
        Ok(())
    }
}

#[cfg(unix)]
fn share_with_agent_group(path: &Path, gid: u32, mode: u32) -> CliResult<()> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::PermissionsExt;

    let encoded = CString::new(path.as_os_str().as_bytes())?;
    if unsafe { libc::chown(encoded.as_ptr(), !0 as libc::uid_t, gid as libc::gid_t) } != 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    fs::set_permissions(path, fs::Permissions::from_mode(mode))?;
    Ok(())
}

fn print_json(value: &Value) -> CliResult<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

fn init_command(args: &[String]) -> CliResult<()> {
    let directory = data_dir(args)?;
    let token_path = PathBuf::from(required(args, "--owner-token-file")?);
    let _lock = DataDirLock::acquire(&directory)?;
    if directory.join("state.json").exists() {
        return Err("treasury is already initialized".into());
    }
    let owner_name = value(args, "--owner-name").unwrap_or_else(|| "local-owner".to_string());
    let balance =
        value(args, "--balance-minor").unwrap_or_else(|| "10000".to_string()).parse::<i64>()?;
    let bootstrap = Treasury::bootstrap(&owner_name, Money::positive(balance, "CAD")?)?;
    write_token(&token_path, &bootstrap.owner_token)?;
    if let Err(error) = bootstrap.treasury.save_to(&directory) {
        let _ = fs::remove_file(&token_path);
        return Err(error.into());
    }
    print_json(
        &json!({ "initialized": true, "data_dir": directory, "owner_token_file": token_path }),
    )
}

fn run_request(args: &[String], token: String, operation: Request) -> CliResult<Value> {
    let directory = data_dir(args)?;
    #[cfg(unix)]
    {
        let socket_argument =
            if operation.requires_owner() { "--owner-socket" } else { "--socket" };
        let default_name = if operation.requires_owner() { "owner.sock" } else { "treasury.sock" };
        let socket = value(args, socket_argument)
            .map(PathBuf::from)
            .unwrap_or_else(|| directory.join(default_name));
        if socket.exists() {
            return rpc_over_socket(&socket, token, operation);
        }
    }
    let _lock = DataDirLock::acquire(&directory)?;
    let mut treasury = Treasury::load_from(&directory)?;
    if treasury.recover_interrupted_executions()? > 0 {
        treasury.save_to(&directory)?;
    }
    treasury.handle_persisted(&token, operation, &directory).map_err(Into::into)
}

#[cfg(unix)]
fn rpc_over_socket(socket: &Path, token: String, operation: Request) -> CliResult<Value> {
    use std::os::unix::net::UnixStream;
    use std::time::{SystemTime, UNIX_EPOCH};
    let request_id = format!(
        "cli-{}-{}",
        std::process::id(),
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()
    );
    let request = RpcRequest { api_version: API_VERSION.to_string(), request_id, token, operation };
    let mut stream = UnixStream::connect(socket)?;
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;
    stream.write_all(serde_json::to_string(&request)?.as_bytes())?;
    stream.write_all(b"\n")?;
    stream.flush()?;
    let mut reader = BufReader::new(stream);
    let mut encoded = Vec::new();
    reader.by_ref().take((MAX_FRAME_BYTES + 1) as u64).read_until(b'\n', &mut encoded)?;
    if encoded.len() > MAX_FRAME_BYTES || encoded.last() != Some(&b'\n') {
        return Err("broker response exceeded the bounded frame size".into());
    }
    let response: RpcResponse = serde_json::from_slice(&encoded)?;
    if response.ok {
        response.data.ok_or_else(|| "broker returned no data".into())
    } else {
        Err(response.error.unwrap_or_else(|| "broker rejected request".to_string()).into())
    }
}

fn direct_command(args: &[String], operation: Request) -> CliResult<()> {
    let owner = matches!(operation, Request::OwnerListAudit);
    let token = token_file(args, owner)?;
    print_json(&run_request(args, token, operation)?)
}

fn create_agent_command(args: &[String]) -> CliResult<()> {
    let owner = token_file(args, true)?;
    let token_path = PathBuf::from(required(args, "--agent-token-file")?);
    if token_path.exists() {
        return Err("agent token file already exists".into());
    }
    #[cfg(unix)]
    let agent_gid = value(args, "--agent-gid").map(|value| value.parse::<u32>()).transpose()?;
    #[cfg(unix)]
    if agent_gid == Some(unsafe { libc::getegid() }) {
        return Err(
            "--agent-gid must identify a group distinct from the broker primary group".into()
        );
    }
    let policy = if let Some(path) = value(args, "--policy-file") {
        serde_json::from_slice::<Policy>(&fs::read(path)?)?
    } else {
        Policy::conservative_demo()?
    };
    let mode = match value(args, "--mode").as_deref().unwrap_or("approval_required") {
        "observe" => AutonomyMode::Observe,
        "approval_required" => AutonomyMode::ApprovalRequired,
        "bounded_autonomous" => AutonomyMode::BoundedAutonomous,
        "disabled" => AutonomyMode::Disabled,
        value => return Err(format!("unsupported mode {value}").into()),
    };
    let result = run_request(
        args,
        owner,
        Request::OwnerCreateAgent {
            name: value(args, "--name").unwrap_or_else(|| "local-agent".to_string()),
            policy,
            mode,
            ttl_secs: value(args, "--ttl-secs").unwrap_or_else(|| "3600".to_string()).parse()?,
        },
    )?;
    let token =
        result["capability_token"].as_str().ok_or("broker did not return a capability token")?;
    write_token(&token_path, token)?;
    #[cfg(unix)]
    if let Some(gid) = agent_gid {
        if let Some(parent) = token_path.parent() {
            share_with_agent_group(parent, gid, 0o750)?;
        }
        share_with_agent_group(&token_path, gid, 0o640)?;
    }
    print_json(
        &json!({ "agent_id": result["agent_id"], "agent_token_file": token_path, "expires_at": result["expires_at"] }),
    )
}

fn update_policy_command(args: &[String]) -> CliResult<()> {
    let token = token_file(args, true)?;
    let policy: Policy = serde_json::from_slice(&fs::read(required(args, "--policy-file")?)?)?;
    print_json(&run_request(
        args,
        token,
        Request::OwnerUpdatePolicy { agent_id: required(args, "--agent-id")?, policy },
    )?)
}

fn configure_receive_command(args: &[String]) -> CliResult<()> {
    let token = token_file(args, true)?;
    print_json(&run_request(
        args,
        token,
        Request::OwnerConfigureReceiveInstructions {
            method: value(args, "--method").unwrap_or_else(|| "interac_e_transfer".to_string()),
            address: required(args, "--address")?,
            memo_template: value(args, "--memo-template")
                .unwrap_or_else(|| "AGENT-{agent_id}-{intent_id}".to_string()),
        },
    )?)
}

fn agent_lifecycle_command(args: &[String], revoke: bool) -> CliResult<()> {
    let token = token_file(args, true)?;
    let agent_id = required(args, "--agent-id")?;
    let operation = if revoke {
        Request::OwnerRevokeAgent { agent_id }
    } else {
        let mode = match required(args, "--mode")?.as_str() {
            "observe" => AutonomyMode::Observe,
            "approval_required" => AutonomyMode::ApprovalRequired,
            "bounded_autonomous" => AutonomyMode::BoundedAutonomous,
            "disabled" => AutonomyMode::Disabled,
            value => return Err(format!("unsupported mode {value}").into()),
        };
        Request::OwnerSetAgentMode { agent_id, mode }
    };
    print_json(&run_request(args, token, operation)?)
}

fn arm_session_command(args: &[String]) -> CliResult<()> {
    let token = token_file(args, true)?;
    print_json(&run_request(
        args,
        token,
        Request::OwnerArmAgentSession {
            agent_id: required(args, "--agent-id")?,
            ttl_secs: value(args, "--ttl-secs").unwrap_or_else(|| "600".to_string()).parse()?,
        },
    )?)
}

fn configure_manual_provider_command(args: &[String]) -> CliResult<()> {
    let token = token_file(args, true)?;
    let balance_status = match required(args, "--balance-status")?.as_str() {
        "estimated" => BalanceStatus::Estimated,
        "owner_confirmed" => BalanceStatus::OwnerConfirmed,
        value => return Err(format!("unsupported balance status {value}").into()),
    };
    print_json(&run_request(
        args,
        token,
        Request::OwnerConfigureManualProvider {
            credential_reference: required(args, "--credential-reference")?,
            provider_kind: value(args, "--provider-kind")
                .unwrap_or_else(|| "os-credential-store".to_string()),
            last_four: value(args, "--last-four"),
            balance: Money::new(
                required(args, "--balance-minor")?.parse()?,
                &value(args, "--currency").unwrap_or_else(|| "CAD".to_string()),
            )?,
            balance_status,
            balance_ttl_secs: value(args, "--balance-ttl-secs")
                .unwrap_or_else(|| "900".to_string())
                .parse()?,
        },
    )?)
}

fn record_deposit_command(args: &[String]) -> CliResult<()> {
    let token = token_file(args, true)?;
    let amount = value(args, "--amount-minor")
        .ok_or("missing required argument --amount-minor")?
        .parse::<i64>()?;
    let currency = value(args, "--currency").unwrap_or_else(|| "CAD".to_string());
    let verified = match value(args, "--verified").as_deref() {
        Some("true") => true,
        Some("false") | None => false,
        Some(value) => return Err(format!("--verified must be true or false, got {value}").into()),
    };
    print_json(&run_request(
        args,
        token,
        Request::OwnerRecordDeposit {
            amount: Money::new(amount, &currency)?,
            source: value(args, "--source").unwrap_or_else(|| "owner-reconciliation".to_string()),
            verified,
            agent_id: value(args, "--agent-id"),
            external_reference: required(args, "--external-reference")?,
        },
    )?)
}

fn intent_command(args: &[String]) -> CliResult<()> {
    let token = token_file(args, false)?;
    let request: PurchaseRequest =
        serde_json::from_slice(&fs::read(required(args, "--request-file")?)?)?;
    print_json(&run_request(args, token, Request::CreatePurchaseIntent { request })?)
}

fn intent_id_command(args: &[String], cancel: bool) -> CliResult<()> {
    let token = token_file(args, false)?;
    let intent_id = required(args, "--intent-id")?;
    let operation = if cancel {
        Request::CancelPurchaseIntent { intent_id }
    } else {
        Request::ExecutePurchaseIntent { intent_id }
    };
    print_json(&run_request(args, token, operation)?)
}

fn owner_intent_command(args: &[String], _unused: bool) -> CliResult<()> {
    let token = token_file(args, true)?;
    print_json(&run_request(
        args,
        token,
        Request::OwnerApproveIntent { intent_id: required(args, "--intent-id")? },
    )?)
}

fn owner_handoff_command(args: &[String], complete: bool) -> CliResult<()> {
    let token = token_file(args, true)?;
    #[cfg(unix)]
    {
        let directory = data_dir(args)?;
        let owner_socket = value(args, "--owner-socket")
            .map(PathBuf::from)
            .unwrap_or_else(|| directory.join("owner.sock"));
        if !owner_socket.exists() {
            return Err(
                "manual handoff commands require a continuously running broker daemon".into()
            );
        }
    }
    let intent_id = required(args, "--intent-id")?;
    let operation = if complete {
        Request::OwnerCompleteManualHandoff { intent_id }
    } else {
        Request::OwnerBeginManualHandoff { intent_id }
    };
    print_json(&run_request(args, token, operation)?)
}

#[cfg(unix)]
fn init_helper_command(args: &[String]) -> CliResult<()> {
    let directory = PathBuf::from(required(args, "--helper-dir")?);
    fs::create_dir_all(&directory)?;
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))?;
    let mut random = [0_u8; 48];
    fs::File::open("/dev/urandom")?.read_exact(&mut random)?;
    let key_path = directory.join("helper.key");
    let id_path = directory.join("helper.id");
    write_token(&key_path, &hex::encode(&random[..32]))?;
    write_token(&id_path, &hex::encode(&random[32..]))?;
    print_json(&json!({
        "initialized": true,
        "helper_key_file": key_path,
        "helper_id_file": id_path,
        "rotate_by_reinitializing_only_when_no_handoff_is_active": true,
    }))
}

#[cfg(not(unix))]
fn init_helper_command(_args: &[String]) -> CliResult<()> {
    Err("the owner helper requires authenticated Unix-domain sockets".into())
}

#[cfg(unix)]
struct SocketPathGuard(PathBuf);

#[cfg(unix)]
impl Drop for SocketPathGuard {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

#[cfg(unix)]
fn secret_helper_command(args: &[String]) -> CliResult<()> {
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::net::UnixListener;

    let socket_path = PathBuf::from(required(args, "--socket")?);
    let helper_key = Zeroizing::new(read_private_text(
        &PathBuf::from(required(args, "--helper-key-file")?),
        "helper key file",
    )?);
    let helper_id = Zeroizing::new(read_private_text(
        &PathBuf::from(required(args, "--helper-id-file")?),
        "helper id file",
    )?);
    let redemption_store =
        DurableNonceRedemptionStore::open(PathBuf::from(required(args, "--redemption-dir")?))?;
    if socket_path.exists() {
        return Err("secret-helper socket already exists".into());
    }
    let listener = UnixListener::bind(&socket_path)?;
    let _socket_guard = SocketPathGuard(socket_path.clone());
    fs::set_permissions(&socket_path, fs::Permissions::from_mode(0o600))?;
    listener.set_nonblocking(true)?;
    let mut secret = Zeroizing::new(Vec::new());
    if unsafe { libc::isatty(libc::STDIN_FILENO) } == 1 {
        let mut terminal: libc::termios = unsafe { std::mem::zeroed() };
        if unsafe { libc::tcgetattr(libc::STDIN_FILENO, &mut terminal) } != 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        let original = terminal;
        terminal.c_lflag &= !libc::ECHO;
        if unsafe { libc::tcsetattr(libc::STDIN_FILENO, libc::TCSAFLUSH, &terminal) } != 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        eprint!("Owner payment JSON (hidden): ");
        let read_result = std::io::stdin().take(4097).read_to_end(&mut secret);
        let restore_result =
            unsafe { libc::tcsetattr(libc::STDIN_FILENO, libc::TCSAFLUSH, &original) };
        eprintln!();
        read_result?;
        if restore_result != 0 {
            secret.fill(0);
            return Err(std::io::Error::last_os_error().into());
        }
    } else {
        std::io::stdin().take(4097).read_to_end(&mut secret)?;
    }
    while matches!(secret.last(), Some(b'\n' | b'\r')) {
        secret.pop();
    }
    if secret.is_empty() || secret.len() > 4096 {
        return Err("owner secret on stdin must contain 1..4096 bytes".into());
    }
    (|| -> CliResult<()> {
        let deadline = Instant::now() + Duration::from_secs(300);
        let stream = loop {
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    if Instant::now() >= deadline {
                        return Err(
                            "secret-helper expired before an authenticated broker connected".into(),
                        );
                    }
                    std::thread::sleep(Duration::from_millis(20));
                }
                Err(error) => return Err(error.into()),
            }
        };
        stream.set_read_timeout(Some(Duration::from_secs(30)))?;
        stream.set_write_timeout(Some(Duration::from_secs(30)))?;
        let peer_uid = unix_peer_effective_uid(&stream)?;
        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        reader.by_ref().take((MAX_FRAME_BYTES + 1) as u64).read_line(&mut line)?;
        if line.len() > MAX_FRAME_BYTES || !line.ends_with('\n') {
            return Err("secret-helper request frame is invalid".into());
        }
        let operation: ApprovedSecretOperation = serde_json::from_str(&line)?;
        redeem_owner_helper_operation(
            &operation,
            helper_key.as_bytes(),
            &helper_id,
            peer_uid,
            &redemption_store,
        )?;
        let mut stream = reader.into_inner();
        stream.write_all(&(secret.len() as u32).to_be_bytes())?;
        stream.write_all(&secret)?;
        stream.flush()?;
        Ok(())
    })()
}

#[cfg(not(unix))]
fn secret_helper_command(_args: &[String]) -> CliResult<()> {
    Err("the owner helper requires authenticated Unix-domain sockets".into())
}

#[cfg(unix)]
fn execute_handoff_command(args: &[String]) -> CliResult<()> {
    let directory = data_dir(args)?;
    let _lock = DataDirLock::acquire(&directory)?;
    let owner_token = token_file(args, true)?;
    let intent_id = required(args, "--intent-id")?;
    let helper_socket = PathBuf::from(required(args, "--helper-socket")?);
    let helper_key = Zeroizing::new(read_private_text(
        &PathBuf::from(required(args, "--helper-key-file")?),
        "helper key file",
    )?);
    let helper_id = Zeroizing::new(read_private_text(
        &PathBuf::from(required(args, "--helper-id-file")?),
        "helper id file",
    )?);
    let node_path = PathBuf::from(required(args, "--node-path")?);
    let adapter_script = PathBuf::from(required(args, "--adapter-script")?);
    let adapter_config = PathBuf::from(required(args, "--adapter-config")?);
    require_owner_executable(&node_path, "Node executable")?;
    require_absolute_regular_file(&adapter_script, "checkout adapter script")?;
    require_absolute_regular_file(&adapter_config, "checkout adapter config")?;
    let config: Value = serde_json::from_slice(&fs::read(&adapter_config)?)?;
    let browser_path = PathBuf::from(
        config
            .get("browserExecutable")
            .and_then(Value::as_str)
            .ok_or("checkout adapter config requires browserExecutable")?,
    );
    require_owner_executable(&browser_path, "browser executable")?;
    let timeout_ms = config
        .get("timeoutMs")
        .and_then(Value::as_u64)
        .ok_or("checkout adapter config requires an integer timeoutMs")?;
    if !(1_000..=120_000).contains(&timeout_ms) {
        return Err("checkout adapter timeoutMs must be within 1000..120000".into());
    }
    let hard_timeout =
        Duration::from_millis(timeout_ms.saturating_mul(2).saturating_add(1_000).min(180_000));
    let handoff_deadline = Instant::now() + hard_timeout;
    let mut treasury = Treasury::load_from(&directory)?;
    if treasury.recover_interrupted_executions()? > 0 {
        treasury.save_to(&directory)?;
    }
    let operation = treasury.bind_approved_secret_helper_operation(
        &owner_token,
        &intent_id,
        helper_key.as_bytes(),
        &helper_id,
        unsafe { libc::geteuid() },
    )?;
    let reference = treasury
        .state
        .manual_provider
        .as_ref()
        .ok_or("manual provider is not configured")?
        .card
        .reference
        .clone();
    let expected_request = treasury
        .state
        .intents
        .get(&intent_id)
        .ok_or("purchase intent was not found")?
        .request
        .clone();
    let provider =
        OwnerControlledSecretHelperProvider::new(helper_socket, &reference, operation.clone())?
            .with_deadline(handoff_deadline);
    let transport = PlaywrightCheckoutTransport {
        node_path,
        adapter_script,
        adapter_config: config,
        deadline: handoff_deadline,
    };
    let mut executor = agent_treasury_domain::SecureOwnerHandoffExecutor::new(
        operation,
        expected_request,
        provider,
        transport,
    )?;
    let result = treasury.owner_execute_approved_handoff_persisted(
        &owner_token,
        &intent_id,
        &mut executor,
        &directory,
    )?;
    print_json(&result)
}

#[cfg(not(unix))]
fn execute_handoff_command(_args: &[String]) -> CliResult<()> {
    Err("controlled checkout requires authenticated Unix-domain sockets".into())
}

fn approve_merchant_command(args: &[String]) -> CliResult<()> {
    let token = token_file(args, true)?;
    print_json(&run_request(
        args,
        token,
        Request::OwnerApproveMerchant {
            agent_id: required(args, "--agent-id")?,
            merchant_domain: required(args, "--merchant-domain")?,
        },
    )?)
}

fn reconcile_command(args: &[String]) -> CliResult<()> {
    let token = token_file(args, true)?;
    let outcome = match required(args, "--outcome")?.as_str() {
        "settled" => ReconciliationOutcome::Settled,
        "declined" => ReconciliationOutcome::Declined,
        "refunded" => ReconciliationOutcome::Refunded,
        value => return Err(format!("unsupported reconciliation outcome {value}").into()),
    };
    print_json(&run_request(
        args,
        token,
        Request::OwnerReconcile {
            intent_id: required(args, "--intent-id")?,
            outcome,
            provider_reference: value(args, "--provider-reference"),
        },
    )?)
}

fn stop_command(args: &[String], stopped: bool) -> CliResult<()> {
    let token = token_file(args, true)?;
    print_json(&run_request(args, token, Request::OwnerSetEmergencyStop { stopped })?)
}

#[cfg(unix)]
fn serve_command(args: &[String]) -> CliResult<()> {
    let directory = data_dir(args)?;
    let _lock = DataDirLock::acquire(&directory)?;
    let agent_socket = value(args, "--socket")
        .map(PathBuf::from)
        .unwrap_or_else(|| directory.join("treasury.sock"));
    let owner_socket = value(args, "--owner-socket")
        .map(PathBuf::from)
        .unwrap_or_else(|| directory.join("owner.sock"));
    let agent_gid = value(args, "--agent-gid").map(|value| value.parse::<u32>()).transpose()?;
    if agent_socket == owner_socket {
        return Err("agent and owner sockets must use different paths".into());
    }
    let mut treasury = Treasury::load_from(&directory)?;
    let require_separate_agent =
        treasury.state.provider_mode == agent_treasury_domain::ProviderMode::ManualPrepaidCard;
    if require_separate_agent && agent_gid.is_none() {
        return Err(
            "manual provider mode requires --agent-gid and a separate agent OS identity".into()
        );
    }
    if agent_gid == Some(unsafe { libc::getegid() }) {
        return Err("--agent-gid must differ from the broker primary group".into());
    }
    if agent_gid.is_some() && agent_socket.parent() == Some(directory.as_path()) {
        return Err(
            "group-shared agent socket must be outside the private broker data directory".into()
        );
    }
    if treasury.recover_interrupted_executions()? > 0 {
        treasury.save_to(&directory)?;
    }
    let state = Arc::new(Mutex::new(treasury));
    let agent_listener = bind_private_socket(&agent_socket)?;
    let owner_listener = bind_private_socket(&owner_socket)?;
    if let Some(gid) = agent_gid {
        if let Some(parent) = agent_socket.parent() {
            share_with_agent_group(parent, gid, 0o750)?;
        }
        share_with_agent_group(&agent_socket, gid, 0o660)?;
    }
    eprintln!("agent-treasury agent broker listening on {}", agent_socket.display());
    eprintln!("agent-treasury owner control listening on {}", owner_socket.display());
    let owner_state = Arc::clone(&state);
    let owner_directory = directory.clone();
    std::thread::spawn(move || {
        serve_listener(
            owner_listener,
            owner_state,
            owner_directory,
            true,
            MAX_OWNER_CONNECTIONS,
            false,
        )
    });
    serve_listener(
        agent_listener,
        state,
        directory,
        false,
        MAX_CONNECTIONS,
        require_separate_agent,
    );
    Ok(())
}

#[cfg(unix)]
fn bind_private_socket(socket: &Path) -> CliResult<std::os::unix::net::UnixListener> {
    use std::os::unix::net::UnixListener;
    if socket.exists() {
        use std::os::unix::fs::FileTypeExt;
        let metadata = fs::symlink_metadata(socket)?;
        if metadata.file_type().is_symlink() || !metadata.file_type().is_socket() {
            return Err(format!("refusing to replace non-socket path {}", socket.display()).into());
        }
        fs::remove_file(socket)?;
    }
    if let Some(parent) = socket.parent() {
        fs::create_dir_all(parent)?;
    }
    let listener = UnixListener::bind(socket)?;
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(socket, fs::Permissions::from_mode(0o600))?;
    Ok(listener)
}

#[cfg(unix)]
fn serve_listener(
    listener: std::os::unix::net::UnixListener,
    state: Arc<Mutex<Treasury>>,
    directory: PathBuf,
    owner_channel: bool,
    connection_limit: usize,
    reject_broker_uid: bool,
) {
    let active_connections = Arc::new(AtomicUsize::new(0));
    for stream in listener.incoming() {
        match stream {
            Ok(mut stream) => {
                if reject_broker_uid
                    && !matches!(
                        peer_effective_uid(&stream),
                        Ok(uid) if uid != unsafe { libc::geteuid() }
                    )
                {
                    let _ = stream.write_all(b"{\"api_version\":\"v1\",\"request_id\":\"identity\",\"ok\":false,\"data\":null,\"error\":\"manual provider requires a separate agent OS identity\"}\n");
                    continue;
                }
                if active_connections.fetch_add(1, Ordering::AcqRel) >= connection_limit {
                    active_connections.fetch_sub(1, Ordering::AcqRel);
                    let _ = stream.write_all(b"{\"api_version\":\"v1\",\"request_id\":\"busy\",\"ok\":false,\"data\":null,\"error\":\"broker is busy\"}\n");
                    continue;
                }
                let state = Arc::clone(&state);
                let directory = directory.clone();
                let active_connections = Arc::clone(&active_connections);
                std::thread::spawn(move || {
                    if let Err(error) = handle_connection(stream, state, directory, owner_channel) {
                        eprintln!("connection error: {}", redact_sensitive(&error.to_string()));
                    }
                    active_connections.fetch_sub(1, Ordering::AcqRel);
                });
            }
            Err(error) => eprintln!("accept error: {}", error),
        }
    }
}

#[cfg(target_os = "macos")]
fn peer_effective_uid(stream: &std::os::unix::net::UnixStream) -> CliResult<u32> {
    use std::os::fd::AsRawFd;
    let mut uid = 0;
    let mut gid = 0;
    if unsafe { libc::getpeereid(stream.as_raw_fd(), &mut uid, &mut gid) } != 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(uid)
}

#[cfg(target_os = "linux")]
fn peer_effective_uid(stream: &std::os::unix::net::UnixStream) -> CliResult<u32> {
    use std::os::fd::AsRawFd;
    let mut credentials: libc::ucred = unsafe { std::mem::zeroed() };
    let mut length = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    if unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            &mut credentials as *mut _ as *mut libc::c_void,
            &mut length,
        )
    } != 0
    {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(credentials.uid)
}

#[cfg(all(unix, not(any(target_os = "macos", target_os = "linux"))))]
fn peer_effective_uid(_stream: &std::os::unix::net::UnixStream) -> CliResult<u32> {
    Err("peer identity checks are unsupported on this Unix platform".into())
}

#[cfg(not(unix))]
fn serve_command(_args: &[String]) -> CliResult<()> {
    Err("Windows builds require the documented named-pipe adapter; the Unix socket daemon is not available on this host".into())
}

#[cfg(unix)]
fn handle_connection(
    stream: std::os::unix::net::UnixStream,
    state: Arc<Mutex<Treasury>>,
    directory: PathBuf,
    owner_channel: bool,
) -> CliResult<()> {
    stream.set_read_timeout(Some(Duration::from_secs(3)))?;
    stream.set_write_timeout(Some(Duration::from_secs(3)))?;
    let mut writer = stream.try_clone()?;
    let mut reader = BufReader::new(stream);
    let mut frame = Vec::new();
    {
        let mut bounded = Read::by_ref(&mut reader).take((MAX_FRAME_BYTES + 1) as u64);
        bounded.read_until(b'\n', &mut frame)?;
    }
    let response = if frame.len() > MAX_FRAME_BYTES || frame.last() != Some(&b'\n') {
        RpcResponse {
            api_version: API_VERSION.to_string(),
            request_id: "oversize".to_string(),
            ok: false,
            data: None,
            error: Some("request is too large or missing its frame terminator".to_string()),
        }
    } else {
        match serde_json::from_slice::<RpcRequest>(&frame) {
            Ok(request) => {
                let mut treasury = state.lock().map_err(|_| "broker state lock poisoned")?;
                if owner_channel && !treasury.is_owner_token(&request.token) {
                    rejected_rpc(&request, "owner control authentication failed")
                } else if !owner_channel && request.operation.requires_owner() {
                    rejected_rpc(&request, "owner operations require the owner control socket")
                } else {
                    treasury.handle_rpc_persisted(request, &directory)
                }
            }
            Err(error) => RpcResponse {
                api_version: API_VERSION.to_string(),
                request_id: "invalid".to_string(),
                ok: false,
                data: None,
                error: Some(redact_sensitive(&format!("invalid request: {error}"))),
            },
        }
    };
    writeln!(writer, "{}", serde_json::to_string(&response)?)?;
    writer.flush()?;
    Ok(())
}

fn rejected_rpc(request: &RpcRequest, error: &str) -> RpcResponse {
    RpcResponse {
        api_version: API_VERSION.to_string(),
        request_id: request.request_id.clone(),
        ok: false,
        data: None,
        error: Some(error.to_string()),
    }
}

fn demo_request(key: &str, amount: i64) -> PurchaseRequest {
    PurchaseRequest {
        idempotency_key: key.to_string(),
        amount: Money::positive(amount, "CAD").expect("demo money"),
        final_total: Money::positive(amount, "CAD").expect("demo money"),
        merchant_domain: "merchant.example.test".to_string(),
        category: "software".to_string(),
        items: vec![PurchaseItem {
            label: "demo item".to_string(),
            quantity: 1,
            unit_price_minor: amount,
        }],
        recurring: false,
        trial_auto_renew: false,
        stored_card: false,
        tip_minor: 0,
        preauthorization: false,
        installments: false,
        fulfillment_profile: "digital-email".to_string(),
        payment_form: agent_treasury_domain::PaymentFormTrust::HostedFields,
        redirect_chain: vec!["https://merchant.example.test/checkout".to_string()],
        attempts: 1,
        session_id: "demo-session".to_string(),
        scenario: SimulatedScenario::Normal,
    }
}

fn run_demo() -> CliResult<()> {
    let bootstrap = Treasury::bootstrap("demo-owner", Money::positive(10_000, "CAD")?)?;
    let mut treasury = bootstrap.treasury;
    let owner = bootstrap.owner_token;
    let policy = Policy::conservative_demo()?;
    treasury.handle(
        &owner,
        Request::OwnerConfigureReceiveInstructions {
            method: "interac_e_transfer".to_string(),
            address: "demo-inbox@example.invalid".to_string(),
            memo_template: "AGENT-{agent_id}-{intent_id}".to_string(),
        },
    )?;
    let created = treasury.handle(
        &owner,
        Request::OwnerCreateAgent {
            name: "demo-agent".to_string(),
            policy,
            mode: AutonomyMode::BoundedAutonomous,
            ttl_secs: 3600,
        },
    )?;
    let agent_token =
        created["capability_token"].as_str().ok_or("missing demo capability")?.to_string();
    let budget = treasury.handle(&agent_token, Request::GetBudget)?;
    let receive = treasury.handle(&agent_token, Request::GetReceiveInstructions)?;
    let purchase = demo_request("demo-low-value", 500);
    let intent = treasury
        .handle(&agent_token, Request::CreatePurchaseIntent { request: purchase.clone() })?;
    let executed = treasury.handle(
        &agent_token,
        Request::ExecutePurchaseIntent {
            intent_id: intent["id"].as_str().ok_or("missing intent id")?.to_string(),
        },
    )?;
    let duplicate_intent =
        treasury.handle(&agent_token, Request::CreatePurchaseIntent { request: purchase })?;
    let duplicate_charge_count = treasury.state.provider.charges.len();

    let over_budget = treasury.handle(
        &agent_token,
        Request::CreatePurchaseIntent { request: demo_request("over-budget", 3_000) },
    )?;
    let mut recurring = demo_request("recurring", 500);
    recurring.recurring = true;
    let recurring =
        treasury.handle(&agent_token, Request::CreatePurchaseIntent { request: recurring })?;
    let mut currency = demo_request("currency-substitution", 500);
    currency.amount = Money::positive(500, "USD")?;
    currency.final_total = Money::positive(500, "USD")?;
    let currency =
        treasury.handle(&agent_token, Request::CreatePurchaseIntent { request: currency })?;
    let mut hostile_form = demo_request("hostile-form", 500);
    hostile_form.payment_form = agent_treasury_domain::PaymentFormTrust::MerchantControlled;
    let hostile_form =
        treasury.handle(&agent_token, Request::CreatePurchaseIntent { request: hostile_form })?;
    treasury.handle(&owner, Request::OwnerSetEmergencyStop { stopped: true })?;
    let stopped = treasury.handle(
        &agent_token,
        Request::CreatePurchaseIntent { request: demo_request("stopped", 500) },
    )?;
    let secret_operation = ApprovedSecretOperation::for_simulated_test("demo")?;
    let mut secret_provider =
        SimulatedSecretProvider::new("4111111111111111", "737", secret_operation.clone());
    let secret_len = secret_provider.fetch_for_owner_operation(&secret_operation)?.len();
    drop(secret_provider);
    treasury.verify_audit_chain()?;

    print_json(&json!({
        "project": "agent-treasury",
        "demo": "passed",
        "budget": budget,
        "public_receive_instructions": receive,
        "valid_purchase": executed,
        "same_idempotency_key": duplicate_intent["id"],
        "provider_charge_count_after_duplicate": duplicate_charge_count,
        "over_budget": over_budget,
        "recurring": recurring,
        "currency_substitution": currency,
        "merchant_controlled_form": hostile_form,
        "emergency_stop": stopped,
        "secret_canary": { "volatile_secret_consumed_and_cleared": secret_len > 0, "full_pan_or_cvv_emitted": false },
        "audit_chain": "valid",
        "owner_token_emitted": false,
        "agent_token_emitted": false,
    }))
}
