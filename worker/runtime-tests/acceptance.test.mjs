import worker from "./worker-under-test.mjs";

function equal(actual, expected, label) { if (actual !== expected) throw new Error(`${label}: expected ${expected}, received ${actual}`); }
function request(chainId = 8453) {
  return new Request("https://wallet.example/api/rpc", { method: "POST", headers: { "Content-Type": "application/json", Origin: "https://wallet.example", "Sec-Fetch-Site": "same-origin" }, body: JSON.stringify({ chainId, jsonrpc: "2.0", id: 1, method: "eth_chainId", params: [] }) });
}

export default {
  async test(_controller, env) {
    const observations = [];
    await env.FIXTURE.fetch("https://fixture-control.invalid/reset");
    const started = Date.now();
    for (const [chainId, expected] of [[8453, "0x2105"], [1, "0x1"]]) {
      const response = await worker.fetch(request(chainId));
      equal(response.status, 200, `patched chain ${chainId} status`);
      const body = await response.json();
      equal(body.result, expected, `patched chain ${chainId} result`);
    }
    let state = await (await env.FIXTURE.fetch("https://fixture-control.invalid/state")).json();
    equal(state.calls.length, 2, "successful actual native fetch count");
    if (state.calls.some(call => call.redirect !== "manual")) throw new Error("Worker did not use manual redirects");
    observations.push({ scenario: "actual_worker_chain_reads_native_fetch", chains: [8453, 1], status: 200, outbound_calls: state.calls.length, redirect_mode: "manual", elapsed_ms: Date.now() - started });

    await env.FIXTURE.fetch("https://fixture-control.invalid/reset?mode=redirect");
    const redirected = await worker.fetch(request());
    equal(redirected.status, 503, "redirecting RPC providers rejected");
    state = await (await env.FIXTURE.fetch("https://fixture-control.invalid/state")).json();
    equal(state.calls.length, 2, "one rejected response from each fixed read provider");
    if (state.calls.some(call => call.host === "forbidden-redirect.invalid")) throw new Error("RPC redirect was followed");
    observations.push({ scenario: "rpc_redirects_rejected", status: redirected.status, outbound_calls: state.calls.length, redirected_destination_requests: 0 });

    await env.FIXTURE.fetch("https://fixture-control.invalid/reset?mode=redirect");
    const activity = await worker.fetch(new Request("https://wallet.example/api/activity?chainId=8453&address=0x1111111111111111111111111111111111111111"), { BLOCKSCOUT_API_KEY: "fixture-only-nonsecret-token" });
    equal(activity.status, 200, "activity degrades to partial evidence");
    const activityBody = await activity.json();
    equal(activityBody.partial, true, "activity partial flag");
    equal(activityBody.hashes.length, 0, "redirect did not become trusted candidates");
    state = await (await env.FIXTURE.fetch("https://fixture-control.invalid/state")).json();
    equal(state.calls.length, 2, "fixed activity endpoints attempted once each");
    if (state.calls.some(call => call.host !== "api.blockscout.com" || call.redirect !== "manual" || !call.authorization_present)) throw new Error("Activity request escaped its expected fixed destination or policy");
    observations.push({ scenario: "activity_redirects_rejected", status: activity.status, partial: activityBody.partial, outbound_calls: state.calls.length, redirected_destination_requests: 0, credential_forwarded_to_redirect: false });

    console.log("WALLET_WORKER_EVIDENCE " + JSON.stringify({ scenario: "patched_actual_worker_native_runtime", runtime: "workerd 2026-11-04", mode: "controlled outbound fixture; native workerd fetch is not replaced", checks: observations }));
  }
};
