use cixa_domain::{
    API_VERSION, ApprovedSecretOperation, AutonomyMode, BalanceStatus, Money,
    OwnerHandoffTransport, Policy, ProviderOutcome, PurchaseItem, PurchaseRequest,
    ReconciliationOutcome, Request, RpcRequest, RpcResponse, SecretProvider, SimulatedScenario,
    SimulatedSecretProvider, Treasury, VolatileSecret, capability_fingerprint, redact_sensitive,
};
#[cfg(unix)]
use cixa_domain::{
    DurableNonceRedemptionStore, OwnerControlledSecretHelperProvider,
    redeem_owner_helper_operation, unix_peer_effective_uid,
};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet};
use std::env;
use std::fs::{self, OpenOptions};
use std::hash::{Hash, Hasher};
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
const MAX_AGENT_ADMISSION_ENTRIES: usize = 128;
const MAX_AGENT_REQUESTS_PER_SECOND: u32 = 100;
const MAX_AGENT_CHANNEL_REQUESTS_PER_SECOND: u32 = 200;
const MAX_AGENT_CHANNEL_IN_FLIGHT: u32 = 4;
const MAX_UNAUTHENTICATED_REQUESTS_PER_SECOND: u32 = 20;

struct AgentAdmission {
    state: Mutex<AgentAdmissionState>,
}

struct AgentAdmissionState {
    known_capabilities: HashMap<String, i64>,
    entries: HashMap<u64, AgentAdmissionEntry>,
    window_started: Instant,
    requests: u32,
    in_flight: u32,
    unauthenticated_window_started: Instant,
    unauthenticated_requests: u32,
}

struct AgentAdmissionEntry {
    window_started: Instant,
    requests: u32,
    in_flight: bool,
}

struct AgentChannelGuard {
    admission: Arc<AgentAdmission>,
}

struct AgentCapabilityGuard {
    admission: Arc<AgentAdmission>,
    fingerprint: u64,
}

fn unix_timestamp() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(i64::MAX, |duration| i64::try_from(duration.as_secs()).unwrap_or(i64::MAX))
}

fn admission_fingerprint(fingerprint: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    fingerprint.hash(&mut hasher);
    hasher.finish()
}

impl AgentAdmission {
    fn new(known_capabilities: Vec<(String, i64)>) -> Self {
        Self {
            state: Mutex::new(AgentAdmissionState {
                known_capabilities: known_capabilities.into_iter().collect(),
                entries: HashMap::new(),
                window_started: Instant::now(),
                requests: 0,
                in_flight: 0,
                unauthenticated_window_started: Instant::now(),
                unauthenticated_requests: 0,
            }),
        }
    }

    fn is_known_capability(&self, token: &str) -> bool {
        if token.len() != 64 || !token.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return false;
        }
        let at = unix_timestamp();
        let fingerprint = capability_fingerprint(token);
        self.state.lock().is_ok_and(|state| {
            state.known_capabilities.get(&fingerprint).is_some_and(|expires_at| *expires_at > at)
        })
    }

    fn replace_known_capabilities(&self, capabilities: Vec<(String, i64)>) {
        if let Ok(mut state) = self.state.lock() {
            state.known_capabilities = capabilities.into_iter().collect();
            let active_keys: HashSet<u64> = state
                .known_capabilities
                .keys()
                .map(|fingerprint| admission_fingerprint(fingerprint))
                .collect();
            state
                .entries
                .retain(|fingerprint, entry| entry.in_flight || active_keys.contains(fingerprint));
        }
    }

    fn admit_unauthenticated(&self) -> bool {
        let now = Instant::now();
        let Ok(mut state) = self.state.lock() else {
            return false;
        };
        if now.duration_since(state.unauthenticated_window_started) >= Duration::from_secs(1) {
            state.unauthenticated_window_started = now;
            state.unauthenticated_requests = 0;
        }
        if state.unauthenticated_requests >= MAX_UNAUTHENTICATED_REQUESTS_PER_SECOND {
            return false;
        }
        state.unauthenticated_requests += 1;
        true
    }

    fn admit_channel(self: &Arc<Self>) -> Option<AgentChannelGuard> {
        let now = Instant::now();
        let mut state = self.state.lock().ok()?;
        if now.duration_since(state.window_started) >= Duration::from_secs(1) {
            state.window_started = now;
            state.requests = 0;
        }
        if state.in_flight >= MAX_AGENT_CHANNEL_IN_FLIGHT
            || state.requests >= MAX_AGENT_CHANNEL_REQUESTS_PER_SECOND
        {
            return None;
        }
        state.in_flight += 1;
        state.requests += 1;
        Some(AgentChannelGuard { admission: Arc::clone(self) })
    }

    fn admit_capability(self: &Arc<Self>, token: &str) -> Option<AgentCapabilityGuard> {
        let fingerprint = admission_fingerprint(&capability_fingerprint(token));
        let now = Instant::now();
        let mut state = self.state.lock().ok()?;
        state.entries.retain(|_, entry| {
            entry.in_flight || now.duration_since(entry.window_started) < Duration::from_secs(2)
        });
        if !state.entries.contains_key(&fingerprint)
            && state.entries.len() >= MAX_AGENT_ADMISSION_ENTRIES
        {
            return None;
        }
        let entry = state.entries.entry(fingerprint).or_insert(AgentAdmissionEntry {
            window_started: now,
            requests: 0,
            in_flight: false,
        });
        if now.duration_since(entry.window_started) >= Duration::from_secs(1) {
            entry.window_started = now;
            entry.requests = 0;
        }
        if entry.in_flight || entry.requests >= MAX_AGENT_REQUESTS_PER_SECOND {
            return None;
        }
        entry.in_flight = true;
        entry.requests += 1;
        Some(AgentCapabilityGuard { admission: Arc::clone(self), fingerprint })
    }

    fn admit_authenticated(
        self: &Arc<Self>,
        token: &str,
    ) -> Option<(AgentChannelGuard, AgentCapabilityGuard)> {
        let channel = self.admit_channel()?;
        let capability = self.admit_capability(token)?;
        Some((channel, capability))
    }
}

impl Drop for AgentChannelGuard {
    fn drop(&mut self) {
        if let Ok(mut state) = self.admission.state.lock() {
            state.in_flight = state.in_flight.saturating_sub(1);
        }
    }
}

impl Drop for AgentCapabilityGuard {
    fn drop(&mut self) {
        if let Ok(mut state) = self.admission.state.lock()
            && let Some(entry) = state.entries.get_mut(&self.fingerprint)
        {
            entry.in_flight = false;
        }
    }
}

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
            let lock_path = directory.join("cixa.lock");
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
            fs::set_permissions(directory.join("cixa.lock"), fs::Permissions::from_mode(0o600))?;
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
                .open(directory.join("cixa.lock"))?;
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
        "secret-session" => secret_session_command(&rest),
        "execute-handoff" => execute_handoff_command(&rest),
        "approve-merchant" => approve_merchant_command(&rest),
        "reconcile" => reconcile_command(&rest),
        "stop" => stop_command(&rest, true),
        "resume" => stop_command(&rest, false),
        "audit" => direct_command(&rest, Request::OwnerListAudit),
        "serve" => serve_command(&rest),
        other => Err(format!("unknown command {other}; run `cixa help`").into()),
    }
}

fn print_help() -> CliResult<()> {
    println!(
        "Cixa {}\n\nCommands:\n  demo                              Run the local adversarial demo\n  init --data-dir DIR --owner-token-file FILE\n  create-agent --data-dir DIR --owner-token-file FILE --agent-token-file FILE [--agent-gid GID]\n  update-policy --data-dir DIR --owner-token-file FILE --agent-id ID --policy-file FILE\n  revoke-agent --data-dir DIR --owner-token-file FILE --agent-id ID\n  set-agent-mode --data-dir DIR --owner-token-file FILE --agent-id ID --mode MODE\n  arm-session --data-dir DIR --owner-token-file FILE --agent-id ID --ttl-secs N\n  configure-manual-provider --data-dir DIR --owner-token-file FILE --credential-reference REF --balance-minor N --balance-status estimated|owner_confirmed [--autonomous-checkout true|false]\n  configure-receive --data-dir DIR --owner-token-file FILE --address VALUE\n  record-deposit --data-dir DIR --owner-token-file FILE --amount-minor N --currency CAD --source VALUE --external-reference REF --verified true|false\n  status|budget|capabilities|receive-instructions --data-dir DIR --token-file FILE\n  intent --data-dir DIR --token-file FILE --request-file FILE\n  execute|cancel --data-dir DIR --token-file FILE --intent-id ID\n  approve --data-dir DIR --owner-token-file FILE --intent-id ID\n  begin-handoff|complete-handoff --data-dir DIR --owner-token-file FILE --intent-id ID\n  init-helper --helper-dir DIR\n  secret-helper --socket PATH --helper-key-file FILE --helper-id-file FILE --redemption-dir DIR\n  secret-session --socket PATH --helper-key-file FILE --helper-id-file FILE --redemption-dir DIR [--ttl-secs N] [--max-operations N]\n  execute-handoff --data-dir DIR --owner-token-file FILE --intent-id ID --helper-socket PATH --helper-key-file FILE --helper-id-file FILE --adapter-script FILE --adapter-config FILE --node-path FILE\n  approve-merchant --data-dir DIR --owner-token-file FILE --agent-id ID --merchant-domain DOMAIN\n  reconcile --data-dir DIR --owner-token-file FILE --intent-id ID --outcome settled|declined|refunded [--provider-reference REF]\n  stop|resume --data-dir DIR --owner-token-file FILE\n  audit --data-dir DIR --owner-token-file FILE\n  serve --data-dir DIR [--socket PATH] [--owner-socket PATH] [--agent-gid GID] [--checkout-runtime-dir DIR --checkout-profiles-dir DIR --node-path FILE --adapter-script FILE]\n\nTokens and payment material are read from protected files or stdin, never accepted as command-line values or printed.\nThe broker binds separate agent and owner Unix-domain sockets by default and does not expose a public listener.",
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
    let parent =
        path.parent().filter(|value| !value.as_os_str().is_empty()).unwrap_or(Path::new("."));
    fs::create_dir_all(parent)?;
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
    fs::File::open(parent)?.sync_all()?;
    Ok(())
}

fn new_capability_token() -> CliResult<Zeroizing<String>> {
    Ok(Zeroizing::new(hex::encode(rand::random::<[u8; 32]>())))
}

fn activate_with_prepared_token(
    token_path: &Path,
    agent_gid: Option<u32>,
    activate: impl FnOnce(String) -> CliResult<Value>,
) -> CliResult<Value> {
    let capability_token = new_capability_token()?;
    write_token(token_path, &capability_token)?;
    #[cfg(unix)]
    if let Some(gid) = agent_gid {
        let parent = token_path
            .parent()
            .filter(|value| !value.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        share_with_agent_group(parent, gid, 0o750)?;
        share_with_agent_group(token_path, gid, 0o640)?;
        OpenOptions::new().read(true).write(true).open(token_path)?.sync_all()?;
        fs::File::open(parent)?.sync_all()?;
    }
    #[cfg(not(unix))]
    let _ = agent_gid;
    activate(capability_token.to_string()).map_err(|error| {
        format!(
            "agent activation failed or became uncertain; prepared token retained at {} for owner reconciliation: {error}",
            token_path.display()
        )
        .into()
    })
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
        if (metadata.uid() != 0 && metadata.uid() != unsafe { libc::geteuid() })
            || metadata.permissions().mode() & 0o022 != 0
        {
            return Err(format!(
                "{label} must be root/owner-controlled and not group/world writable"
            )
            .into());
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
fn require_owner_directory(path: &Path, label: &str) -> CliResult<()> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    if !path.is_absolute() {
        return Err(format!("{label} must be an absolute path").into());
    }
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink()
        || !metadata.is_dir()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.permissions().mode() & 0o077 != 0
    {
        return Err(format!("{label} must be a private owner-controlled directory").into());
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
    fn oversized_responses_are_replaced_with_a_bounded_error() {
        let response = RpcResponse {
            api_version: API_VERSION.to_string(),
            request_id: "large-response".to_string(),
            ok: true,
            data: Some(json!({ "payload": "x".repeat(MAX_FRAME_BYTES) })),
            error: None,
        };
        let mut encoded = Vec::new();

        write_response(&mut encoded, &response).unwrap();

        assert!(encoded.len() <= MAX_FRAME_BYTES);
        let decoded: RpcResponse = serde_json::from_slice(&encoded[..encoded.len() - 1]).unwrap();
        assert!(!decoded.ok);
        assert_eq!(decoded.request_id, "oversize");
        assert_eq!(decoded.error.as_deref(), Some("response exceeds the broker frame limit"));

        let attacker_controlled_id = RpcResponse {
            api_version: API_VERSION.to_string(),
            request_id: "r".repeat(MAX_FRAME_BYTES),
            ok: false,
            data: None,
            error: Some("rejected".to_string()),
        };
        encoded.clear();
        write_response(&mut encoded, &attacker_controlled_id).unwrap();
        assert!(encoded.len() <= MAX_FRAME_BYTES);
        let decoded: RpcResponse = serde_json::from_slice(&encoded[..encoded.len() - 1]).unwrap();
        assert_eq!(decoded.request_id, "oversize");
    }

    #[test]
    fn agent_admission_allows_only_one_in_flight_request_per_capability() {
        let admission = Arc::new(AgentAdmission::new(Vec::new()));
        let first = admission.admit_capability("agent-capability").expect("first request admitted");
        assert!(admission.admit_capability("agent-capability").is_none());
        assert!(admission.admit_capability("different-capability").is_some());
        drop(first);
        assert!(admission.admit_capability("agent-capability").is_some());

        let guards: Vec<_> =
            (0..MAX_AGENT_CHANNEL_IN_FLIGHT).map(|_| admission.admit_channel().unwrap()).collect();
        assert!(admission.admit_channel().is_none());
        drop(guards);
    }

    #[test]
    fn unauthenticated_channel_admission_cannot_fill_capability_entries() {
        let token = "a".repeat(64);
        let admission = Arc::new(AgentAdmission::new(vec![(
            capability_fingerprint(&token),
            unix_timestamp() + 60,
        )]));
        for _ in 0..MAX_AGENT_ADMISSION_ENTRIES {
            admission.admit_unauthenticated();
        }
        assert!(admission.state.lock().unwrap().entries.is_empty());
        assert!(admission.is_known_capability(&token));
        assert!(admission.admit_channel().is_some());
        assert!(admission.admit_capability(&token).is_some());
    }

    #[test]
    fn authenticated_forbidden_requests_consume_capability_admission() {
        let token = "b".repeat(64);
        let admission = Arc::new(AgentAdmission::new(vec![(
            capability_fingerprint(&token),
            unix_timestamp() + 60,
        )]));
        for _ in 0..MAX_AGENT_REQUESTS_PER_SECOND {
            drop(admission.admit_authenticated(&token).unwrap());
        }
        assert!(admission.admit_authenticated(&token).is_none());
        admission.replace_known_capabilities(vec![(
            capability_fingerprint(&token),
            unix_timestamp() + 60,
        )]);
        assert!(admission.admit_authenticated(&token).is_none());
    }

    #[test]
    fn expired_capabilities_are_not_classified_as_known() {
        let token = "c".repeat(64);
        let admission =
            AgentAdmission::new(vec![(capability_fingerprint(&token), unix_timestamp() - 1)]);
        assert!(!admission.is_known_capability(&token));
        assert!(!admission.is_known_capability(&"x".repeat(MAX_FRAME_BYTES)));
    }

    #[test]
    fn prepared_cli_token_is_durable_before_activation_and_retained_on_uncertainty() {
        let directory = std::env::temp_dir().join(format!(
            "cixa-prepared-token-{}-{}",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        fs::create_dir_all(&directory).unwrap();
        let token_path = directory.join("agent.token");
        let error = activate_with_prepared_token(&token_path, None, |token| {
            assert_eq!(fs::read_to_string(&token_path).unwrap().trim(), token);
            Err("injected response loss after activation started".into())
        })
        .unwrap_err();
        assert!(error.to_string().contains("prepared token retained"));
        assert_eq!(fs::read_to_string(&token_path).unwrap().trim().len(), 64);
        fs::remove_dir_all(directory).unwrap();
    }

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
) -> cixa_domain::Result<(ExitStatus, Zeroizing<Vec<u8>>)> {
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
                    return Err(cixa_domain::TreasuryError::Conflict(
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
                    cixa_domain::TreasuryError::Conflict(
                        "controlled checkout adapter could not be reaped".to_string(),
                    )
                })?;
                return Ok((status, output));
            }
        }
        std::thread::sleep(Duration::from_millis(10));
    }

    terminate_child_group(child, tracker);
    Err(cixa_domain::TreasuryError::Conflict(
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
    ) -> cixa_domain::Result<ProviderOutcome> {
        let secret: CheckoutSecret<'_> =
            serde_json::from_slice(secret.as_bytes()).map_err(|_| {
                cixa_domain::TreasuryError::Invalid(
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
            return Err(cixa_domain::TreasuryError::Invalid(
                "owner secret fields are missing or outside their size limits".to_string(),
            ));
        }
        let input = CheckoutAdapterInput { config: &self.adapter_config, request, secret };
        let mut encoded = Zeroizing::new(serde_json::to_vec(&input)?);
        encoded.push(b'\n');
        if encoded.len() > 16 * 1024 {
            return Err(cixa_domain::TreasuryError::Invalid(
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
                return Err(cixa_domain::TreasuryError::Conflict(
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
                return Err(cixa_domain::TreasuryError::Conflict(
                    "checkout adapter stdout is unavailable".to_string(),
                ));
            }
        };
        let remaining = self.deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            terminate_child_group(&mut child, &mut tracker);
            return Err(cixa_domain::TreasuryError::Conflict(
                "owner handoff deadline expired before adapter completion".to_string(),
            ));
        }
        let (status, output) = collect_child_output(&mut child, &mut tracker, stdout, remaining)?;
        if !status.success() {
            return Err(cixa_domain::TreasuryError::Conflict(
                "controlled checkout adapter failed; payment outcome is unknown".to_string(),
            ));
        }
        let _: ProviderOutcome = serde_json::from_slice(&output).map_err(|_| {
            cixa_domain::TreasuryError::Conflict(
                "controlled checkout adapter returned an invalid sanitized outcome".to_string(),
            )
        })?;
        Ok(ProviderOutcome::Unknown {
            reason: "browser output is not authenticated provider evidence; owner reconciliation required"
                .to_string(),
        })
    }

    fn cleanup(&mut self) -> cixa_domain::Result<()> {
        Ok(())
    }
}

#[cfg(unix)]
#[derive(Clone)]
struct ControlledCheckoutRuntime {
    helper_socket: PathBuf,
    helper_key_file: PathBuf,
    helper_id_file: PathBuf,
    node_path: PathBuf,
    adapter_script: PathBuf,
    profiles_directory: PathBuf,
}

#[cfg(unix)]
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ControlledCheckoutProfile {
    profile_version: u32,
    merchant_domain: String,
    config: Value,
}

#[cfg(unix)]
impl ControlledCheckoutRuntime {
    fn from_args(args: &[String]) -> CliResult<Option<Self>> {
        let runtime_dir = value(args, "--checkout-runtime-dir");
        let profiles_directory = value(args, "--checkout-profiles-dir");
        let node_path = value(args, "--node-path");
        let adapter_script = value(args, "--adapter-script");
        let configured = [
            runtime_dir.as_ref(),
            profiles_directory.as_ref(),
            node_path.as_ref(),
            adapter_script.as_ref(),
        ]
        .iter()
        .filter(|value| value.is_some())
        .count();
        if configured == 0 {
            return Ok(None);
        }
        if configured != 4 {
            return Err("controlled checkout requires --checkout-runtime-dir, --checkout-profiles-dir, --node-path, and --adapter-script together".into());
        }
        let runtime_dir = fs::canonicalize(PathBuf::from(runtime_dir.unwrap()))?;
        let profiles_directory = fs::canonicalize(PathBuf::from(profiles_directory.unwrap()))?;
        let node_path = fs::canonicalize(PathBuf::from(node_path.unwrap()))?;
        let adapter_script = fs::canonicalize(PathBuf::from(adapter_script.unwrap()))?;
        require_owner_directory(&runtime_dir, "checkout runtime directory")?;
        require_owner_directory(&profiles_directory, "checkout profiles directory")?;
        require_owner_executable(&node_path, "Node executable")?;
        require_absolute_regular_file(&adapter_script, "checkout adapter script")?;
        let helper_key_file = runtime_dir.join("helper.key");
        let helper_id_file = runtime_dir.join("helper.id");
        let _ = read_private_text(&helper_key_file, "helper key file")?;
        let _ = read_private_text(&helper_id_file, "helper id file")?;
        Ok(Some(Self {
            helper_socket: runtime_dir.join("session.sock"),
            helper_key_file,
            helper_id_file,
            node_path,
            adapter_script,
            profiles_directory,
        }))
    }

    fn load_profile(&self, request: &PurchaseRequest) -> CliResult<Value> {
        let mut matched: Option<(PathBuf, Value)> = None;
        let mut entries = 0_usize;
        for entry in fs::read_dir(&self.profiles_directory)? {
            let path = entry?.path();
            if path.extension().and_then(|value| value.to_str()) != Some("json") {
                continue;
            }
            entries += 1;
            if entries > 64 {
                return Err("checkout profile limit exceeded".into());
            }
            require_absolute_regular_file(&path, "checkout profile")?;
            let bytes = fs::read(&path)?;
            if bytes.len() > 64 * 1024 {
                return Err("checkout profile exceeds 64 KiB".into());
            }
            let profile: ControlledCheckoutProfile = serde_json::from_slice(&bytes)?;
            if profile.profile_version != 1 {
                return Err("checkout profile version is unsupported".into());
            }
            if profile.merchant_domain == request.merchant_domain {
                if matched.is_some() {
                    return Err("multiple checkout profiles match the merchant".into());
                }
                matched = Some((path, profile.config));
            }
        }
        let (path, mut config) = matched.ok_or_else(|| {
            format!("no owner-approved checkout profile exists for {}", request.merchant_domain)
        })?;
        let checkout_url = request
            .redirect_chain
            .first()
            .ok_or("controlled checkout request has no checkout URL")?;
        let object = config.as_object_mut().ok_or_else(|| {
            format!("checkout profile {} config must be an object", path.display())
        })?;
        object.insert("checkoutUrl".to_string(), Value::String(checkout_url.clone()));
        let browser_path = object
            .get("browserExecutable")
            .and_then(Value::as_str)
            .ok_or("checkout profile requires browserExecutable")?;
        require_owner_executable(Path::new(browser_path), "browser executable")?;
        Ok(config)
    }

    fn execute(
        &self,
        treasury: &mut Treasury,
        agent_token: &str,
        intent_id: &str,
        directory: &Path,
    ) -> CliResult<Value> {
        let expected_request = treasury
            .state
            .intents
            .get(intent_id)
            .ok_or("purchase intent was not found")?
            .request
            .clone();
        let config = self.load_profile(&expected_request)?;
        let timeout_ms = config
            .get("timeoutMs")
            .and_then(Value::as_u64)
            .ok_or("checkout profile requires an integer timeoutMs")?;
        if !(1_000..=120_000).contains(&timeout_ms) {
            return Err("checkout profile timeoutMs must be within 1000..120000".into());
        }
        let hard_timeout =
            Duration::from_millis(timeout_ms.saturating_mul(2).saturating_add(1_000).min(180_000));
        let deadline = Instant::now() + hard_timeout;
        let helper_key =
            Zeroizing::new(read_private_text(&self.helper_key_file, "helper key file")?);
        let helper_id = Zeroizing::new(read_private_text(&self.helper_id_file, "helper id file")?);
        let operation = treasury.bind_controlled_secret_helper_operation(
            agent_token,
            intent_id,
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
        let provider = OwnerControlledSecretHelperProvider::new(
            self.helper_socket.clone(),
            &reference,
            operation.clone(),
        )?
        .with_deadline(deadline);
        let transport = PlaywrightCheckoutTransport {
            node_path: self.node_path.clone(),
            adapter_script: self.adapter_script.clone(),
            adapter_config: config,
            deadline,
        };
        let mut executor = cixa_domain::SecureOwnerHandoffExecutor::new(
            operation,
            expected_request,
            provider,
            transport,
        )?;
        treasury
            .execute_controlled_checkout_persisted(agent_token, intent_id, &mut executor, directory)
            .map_err(Into::into)
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
        let default_name = if operation.requires_owner() { "owner.sock" } else { "cixa.sock" };
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
    let read_timeout = if matches!(operation, Request::ExecutePurchaseIntent { .. }) {
        Duration::from_secs(180)
    } else {
        Duration::from_secs(5)
    };
    let request = RpcRequest { api_version: API_VERSION.to_string(), request_id, token, operation };
    let mut stream = UnixStream::connect(socket)?;
    stream.set_read_timeout(Some(read_timeout))?;
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
    #[cfg(not(unix))]
    let agent_gid = None;
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
    let name = value(args, "--name").unwrap_or_else(|| "local-agent".to_string());
    let ttl_secs = value(args, "--ttl-secs").unwrap_or_else(|| "3600".to_string()).parse()?;
    let result = activate_with_prepared_token(&token_path, agent_gid, |capability_token| {
        run_request(
            args,
            owner,
            Request::OwnerCreateAgentPrepared { name, policy, mode, ttl_secs, capability_token },
        )
    })?;
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
            autonomous_checkout: match value(args, "--autonomous-checkout").as_deref() {
                Some("true") => true,
                Some("false") | None => false,
                Some(value) => {
                    return Err(format!(
                        "--autonomous-checkout must be true or false, got {value}"
                    )
                    .into());
                }
            },
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
fn read_owner_secret() -> CliResult<Zeroizing<Vec<u8>>> {
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
    Ok(secret)
}

#[cfg(unix)]
fn serve_secret_helper(
    args: &[String],
    ttl_secs: u64,
    max_operations: u32,
    require_operation: bool,
) -> CliResult<()> {
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
    let secret = read_owner_secret()?;
    let deadline = Instant::now() + Duration::from_secs(ttl_secs);
    let mut completed = 0_u32;
    while completed < max_operations {
        let stream = loop {
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    if Instant::now() >= deadline {
                        if require_operation && completed == 0 {
                            return Err(
                                "secret-helper expired before an authenticated broker connected"
                                    .into(),
                            );
                        }
                        return Ok(());
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
        completed += 1;
    }
    Ok(())
}

#[cfg(unix)]
fn secret_helper_command(args: &[String]) -> CliResult<()> {
    serve_secret_helper(args, 300, 1, true)
}

#[cfg(unix)]
fn secret_session_command(args: &[String]) -> CliResult<()> {
    let ttl_secs = value(args, "--ttl-secs").unwrap_or_else(|| "600".to_string()).parse::<u64>()?;
    let max_operations =
        value(args, "--max-operations").unwrap_or_else(|| "10".to_string()).parse::<u32>()?;
    if !(60..=3600).contains(&ttl_secs) {
        return Err("secret-session ttl must be within 60..3600 seconds".into());
    }
    if !(1..=100).contains(&max_operations) {
        return Err("secret-session max operations must be within 1..100".into());
    }
    serve_secret_helper(args, ttl_secs, max_operations, false)
}

#[cfg(not(unix))]
fn secret_helper_command(_args: &[String]) -> CliResult<()> {
    Err("the owner helper requires authenticated Unix-domain sockets".into())
}

#[cfg(not(unix))]
fn secret_session_command(_args: &[String]) -> CliResult<()> {
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
    let mut executor = cixa_domain::SecureOwnerHandoffExecutor::new(
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
    let checkout_runtime = ControlledCheckoutRuntime::from_args(args)?.map(Arc::new);
    let agent_socket =
        value(args, "--socket").map(PathBuf::from).unwrap_or_else(|| directory.join("cixa.sock"));
    let owner_socket = value(args, "--owner-socket")
        .map(PathBuf::from)
        .unwrap_or_else(|| directory.join("owner.sock"));
    let agent_gid = value(args, "--agent-gid").map(|value| value.parse::<u32>()).transpose()?;
    if agent_socket == owner_socket {
        return Err("agent and owner sockets must use different paths".into());
    }
    let mut treasury = Treasury::load_from(&directory)?;
    let require_separate_agent =
        treasury.state.provider_mode == cixa_domain::ProviderMode::ManualPrepaidCard;
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
    let agent_admission = Arc::new(AgentAdmission::new(treasury.active_agent_capabilities()));
    let state = Arc::new(Mutex::new(treasury));
    let agent_listener = bind_private_socket(&agent_socket)?;
    let owner_listener = bind_private_socket(&owner_socket)?;
    if let Some(gid) = agent_gid {
        if let Some(parent) = agent_socket.parent() {
            share_with_agent_group(parent, gid, 0o750)?;
        }
        share_with_agent_group(&agent_socket, gid, 0o660)?;
    }
    eprintln!("Cixa agent broker listening on {}", agent_socket.display());
    eprintln!("Cixa owner control listening on {}", owner_socket.display());
    let owner_state = Arc::clone(&state);
    let owner_directory = directory.clone();
    let owner_admission = Arc::clone(&agent_admission);
    std::thread::spawn(move || {
        serve_listener(
            owner_listener,
            ServeListenerContext {
                state: owner_state,
                directory: owner_directory,
                owner_channel: true,
                connection_limit: MAX_OWNER_CONNECTIONS,
                reject_broker_uid: false,
                agent_admission: Some(owner_admission),
                checkout_runtime: None,
            },
        )
    });
    serve_listener(
        agent_listener,
        ServeListenerContext {
            state,
            directory,
            owner_channel: false,
            connection_limit: MAX_CONNECTIONS,
            reject_broker_uid: require_separate_agent,
            agent_admission: Some(agent_admission),
            checkout_runtime,
        },
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
struct ServeListenerContext {
    state: Arc<Mutex<Treasury>>,
    directory: PathBuf,
    owner_channel: bool,
    connection_limit: usize,
    reject_broker_uid: bool,
    agent_admission: Option<Arc<AgentAdmission>>,
    checkout_runtime: Option<Arc<ControlledCheckoutRuntime>>,
}

#[cfg(unix)]
fn serve_listener(listener: std::os::unix::net::UnixListener, context: ServeListenerContext) {
    let active_connections = Arc::new(AtomicUsize::new(0));
    for stream in listener.incoming() {
        match stream {
            Ok(mut stream) => {
                let peer_uid = peer_effective_uid(&stream).ok();
                if context.reject_broker_uid
                    && !matches!(
                        peer_uid,
                        Some(uid) if uid != unsafe { libc::geteuid() }
                    )
                {
                    let _ = stream.write_all(b"{\"api_version\":\"v1\",\"request_id\":\"identity\",\"ok\":false,\"data\":null,\"error\":\"manual provider requires a separate agent OS identity\"}\n");
                    continue;
                }
                if active_connections.fetch_add(1, Ordering::AcqRel) >= context.connection_limit {
                    active_connections.fetch_sub(1, Ordering::AcqRel);
                    let _ = stream.write_all(b"{\"api_version\":\"v1\",\"request_id\":\"busy\",\"ok\":false,\"data\":null,\"error\":\"broker is busy\"}\n");
                    continue;
                }
                let state = Arc::clone(&context.state);
                let directory = context.directory.clone();
                let active_connections = Arc::clone(&active_connections);
                let owner_channel = context.owner_channel;
                let agent_admission = context.agent_admission.clone();
                let checkout_runtime = context.checkout_runtime.clone();
                std::thread::spawn(move || {
                    if let Err(error) = handle_connection(
                        stream,
                        state,
                        directory,
                        owner_channel,
                        agent_admission,
                        checkout_runtime,
                        peer_uid,
                    ) {
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
    agent_admission: Option<Arc<AgentAdmission>>,
    checkout_runtime: Option<Arc<ControlledCheckoutRuntime>>,
    peer_uid: Option<u32>,
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
                if owner_channel {
                    let mut treasury = state.lock().map_err(|_| "broker state lock poisoned")?;
                    if !treasury.is_owner_token(&request.token) {
                        rejected_rpc(&request, "owner control authentication failed")
                    } else {
                        let response = treasury.handle_rpc_persisted(request, &directory);
                        if let Some(admission) = agent_admission.as_ref() {
                            admission
                                .replace_known_capabilities(treasury.active_agent_capabilities());
                        }
                        response
                    }
                } else if state
                    .lock()
                    .map_err(|_| "broker state lock poisoned")?
                    .state
                    .provider_mode
                    == cixa_domain::ProviderMode::ManualPrepaidCard
                    && !matches!(peer_uid, Some(uid) if uid != unsafe { libc::geteuid() })
                {
                    rejected_rpc(&request, "manual provider requires a separate agent OS identity")
                } else if let Some(admission) = agent_admission.as_ref() {
                    if !admission.is_known_capability(&request.token) {
                        let admitted = admission.admit_unauthenticated();
                        rejected_rpc(
                            &request,
                            if admitted {
                                "agent capability authentication failed"
                            } else {
                                "unauthenticated agent request limit exceeded"
                            },
                        )
                    } else {
                        match admission.admit_authenticated(&request.token) {
                            Some((_channel_guard, _capability_guard)) => {
                                if request.operation.requires_owner() {
                                    rejected_rpc(
                                        &request,
                                        "owner operations require the owner control socket",
                                    )
                                } else {
                                    let mut treasury =
                                        state.lock().map_err(|_| "broker state lock poisoned")?;
                                    let controlled_intent = match &request.operation {
                                        Request::ExecutePurchaseIntent { intent_id }
                                            if treasury.controlled_checkout_enabled() =>
                                        {
                                            Some(intent_id.clone())
                                        }
                                        _ => None,
                                    };
                                    if let Some(intent_id) = controlled_intent {
                                        let request_id = request.request_id.clone();
                                        let result = checkout_runtime
                                            .as_ref()
                                            .ok_or_else(|| {
                                                "controlled checkout runtime is not configured"
                                                    .to_string()
                                            })
                                            .and_then(|runtime| {
                                                runtime
                                                    .execute(
                                                        &mut treasury,
                                                        &request.token,
                                                        &intent_id,
                                                        &directory,
                                                    )
                                                    .map_err(|error| error.to_string())
                                            });
                                        match result {
                                            Ok(data) => RpcResponse {
                                                api_version: API_VERSION.to_string(),
                                                request_id,
                                                ok: true,
                                                data: Some(data),
                                                error: None,
                                            },
                                            Err(error) => RpcResponse {
                                                api_version: API_VERSION.to_string(),
                                                request_id,
                                                ok: false,
                                                data: None,
                                                error: Some(redact_sensitive(&error)),
                                            },
                                        }
                                    } else {
                                        treasury.handle_rpc_persisted(request, &directory)
                                    }
                                }
                            }
                            None => rejected_rpc(&request, "agent request limit exceeded"),
                        }
                    }
                } else {
                    rejected_rpc(&request, "agent admission is unavailable")
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
    write_response(&mut writer, &response)?;
    Ok(())
}

fn write_response(writer: &mut impl Write, response: &RpcResponse) -> CliResult<()> {
    let mut encoded = serde_json::to_vec(response)?;
    if encoded.len() + 1 > MAX_FRAME_BYTES {
        encoded = serde_json::to_vec(&RpcResponse {
            api_version: API_VERSION.to_string(),
            request_id: "oversize".to_string(),
            ok: false,
            data: None,
            error: Some("response exceeds the broker frame limit".to_string()),
        })?;
    }
    writer.write_all(&encoded)?;
    writer.write_all(b"\n")?;
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
        payment_form: cixa_domain::PaymentFormTrust::HostedFields,
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
    hostile_form.payment_form = cixa_domain::PaymentFormTrust::MerchantControlled;
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
        "project": "cixa",
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
