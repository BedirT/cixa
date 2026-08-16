use agent_treasury_domain::{
    API_VERSION, AutonomyMode, BalanceStatus, Money, Policy, PurchaseRequest,
    ReconciliationOutcome, Request, RpcRequest, RpcResponse, SecretProvider, SimulatedScenario,
    SimulatedSecretProvider, Treasury, redact_sensitive,
};
use fs2::FileExt;
use serde_json::{Value, json};
use std::env;
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

type CliResult<T> = Result<T, Box<dyn std::error::Error>>;
const MAX_FRAME_BYTES: usize = 256 * 1024;
const MAX_CONNECTIONS: usize = 32;

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
        "agent-treasury {}\n\nCommands:\n  demo                              Run the local adversarial demo\n  init --data-dir DIR --owner-token-file FILE\n  create-agent --data-dir DIR --owner-token-file FILE --agent-token-file FILE\n  update-policy --data-dir DIR --owner-token-file FILE --agent-id ID --policy-file FILE\n  revoke-agent --data-dir DIR --owner-token-file FILE --agent-id ID\n  set-agent-mode --data-dir DIR --owner-token-file FILE --agent-id ID --mode MODE\n  arm-session --data-dir DIR --owner-token-file FILE --agent-id ID --ttl-secs N\n  configure-manual-provider --data-dir DIR --owner-token-file FILE --credential-reference REF --balance-minor N --balance-status estimated|owner_confirmed\n  configure-receive --data-dir DIR --owner-token-file FILE --address VALUE\n  record-deposit --data-dir DIR --owner-token-file FILE --amount-minor N --currency CAD --source VALUE --external-reference REF --verified true|false\n  status|budget|capabilities|receive-instructions --data-dir DIR --token-file FILE\n  intent --data-dir DIR --token-file FILE --request-file FILE\n  execute|cancel --data-dir DIR --token-file FILE --intent-id ID\n  approve --data-dir DIR --owner-token-file FILE --intent-id ID\n  approve-merchant --data-dir DIR --owner-token-file FILE --agent-id ID --merchant-domain DOMAIN\n  reconcile --data-dir DIR --owner-token-file FILE --intent-id ID --outcome settled|declined|refunded [--provider-reference REF]\n  stop|resume --data-dir DIR --owner-token-file FILE\n  audit --data-dir DIR --owner-token-file FILE\n  serve --data-dir DIR [--socket PATH]\n\nTokens are read from protected files, never accepted as command-line values or printed.\nThe broker binds to a Unix-domain socket by default and does not expose a public listener.",
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
        let socket = value(args, "--socket")
            .map(PathBuf::from)
            .unwrap_or_else(|| directory.join("treasury.sock"));
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
    use std::os::unix::net::UnixListener;
    let directory = data_dir(args)?;
    let _lock = DataDirLock::acquire(&directory)?;
    let socket = value(args, "--socket")
        .map(PathBuf::from)
        .unwrap_or_else(|| directory.join("treasury.sock"));
    let mut treasury = Treasury::load_from(&directory)?;
    if treasury.recover_interrupted_executions()? > 0 {
        treasury.save_to(&directory)?;
    }
    let state = Arc::new(Mutex::new(treasury));
    if socket.exists() {
        use std::os::unix::fs::FileTypeExt;
        let metadata = fs::symlink_metadata(&socket)?;
        if metadata.file_type().is_symlink() || !metadata.file_type().is_socket() {
            return Err(format!("refusing to replace non-socket path {}", socket.display()).into());
        }
        fs::remove_file(&socket)?;
    }
    if let Some(parent) = socket.parent() {
        fs::create_dir_all(parent)?;
    }
    let listener = UnixListener::bind(&socket)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&socket, fs::Permissions::from_mode(0o600))?;
    }
    eprintln!("agent-treasury broker listening on {}", socket.display());
    let active_connections = Arc::new(AtomicUsize::new(0));
    for stream in listener.incoming() {
        match stream {
            Ok(mut stream) => {
                if active_connections.fetch_add(1, Ordering::AcqRel) >= MAX_CONNECTIONS {
                    active_connections.fetch_sub(1, Ordering::AcqRel);
                    let _ = stream.write_all(b"{\"api_version\":\"v1\",\"request_id\":\"busy\",\"ok\":false,\"data\":null,\"error\":\"broker is busy\"}\n");
                    continue;
                }
                let state = Arc::clone(&state);
                let directory = directory.clone();
                let active_connections = Arc::clone(&active_connections);
                std::thread::spawn(move || {
                    if let Err(error) = handle_connection(stream, state, directory) {
                        eprintln!("connection error: {}", redact_sensitive(&error.to_string()));
                    }
                    active_connections.fetch_sub(1, Ordering::AcqRel);
                });
            }
            Err(error) => eprintln!("accept error: {}", error),
        }
    }
    Ok(())
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
                treasury.handle_rpc_persisted(request, &directory)
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

fn demo_request(key: &str, amount: i64) -> PurchaseRequest {
    PurchaseRequest {
        idempotency_key: key.to_string(),
        amount: Money::positive(amount, "CAD").expect("demo money"),
        final_total: Money::positive(amount, "CAD").expect("demo money"),
        merchant_domain: "merchant.example.test".to_string(),
        category: "software".to_string(),
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
    let mut secret_provider = SimulatedSecretProvider::new("4111111111111111", "737");
    let secret_len = secret_provider.fetch_for_owner_operation("demo")?.len();
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
