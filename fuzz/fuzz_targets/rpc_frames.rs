#![no_main]

use agent_treasury_domain::RpcRequest;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if data.len() <= 256 * 1024 {
        let _ = serde_json::from_slice::<RpcRequest>(data);
    }
});
